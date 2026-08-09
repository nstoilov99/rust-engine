use super::pass_node::{PassBuilder, PassContext, PassIndex, PassNode};
use super::resource::{ResourceDesc, ResourceId, ResourceKind, ResourceTable};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::command_buffer::PrimaryAutoCommandBuffer;
use vulkano::image::view::ImageView;

#[derive(Debug)]
pub enum GraphError {
    CycleDetected,
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::CycleDetected => write!(f, "Dependency cycle detected in render graph"),
        }
    }
}

impl std::error::Error for GraphError {}

/// A pass's execution attached at registration time (`add_pass_with`). The
/// `'exec` lifetime lets per-frame graphs capture references to renderer
/// state; a long-lived graph would use `'static` executors.
pub type PassExecutor<'exec> =
    Box<dyn FnMut(&mut PassContext<'_>) -> Result<(), Box<dyn std::error::Error>> + 'exec>;

pub struct RenderGraph<'exec> {
    pub(crate) passes: Vec<PassNode>,
    /// Parallel to `passes`. `None` = declared with `add_pass` (ordering
    /// only) — executing such a pass is a hard error, never a silent no-op.
    executors: Vec<Option<PassExecutor<'exec>>>,
    pub(crate) resources: ResourceTable,
    pub(crate) transient_resources: Vec<ResourceId>,
    compiled_order: Vec<PassIndex>,
    output_resources: HashSet<ResourceId>,
    culling_enabled: bool,
}

impl Default for RenderGraph<'_> {
    fn default() -> Self {
        Self {
            passes: Vec::new(),
            executors: Vec::new(),
            resources: ResourceTable::new(),
            transient_resources: Vec::new(),
            compiled_order: Vec::new(),
            output_resources: HashSet::new(),
            culling_enabled: false,
        }
    }
}

impl<'exec> RenderGraph<'exec> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn import_image(&mut self, name: &str, image: Arc<ImageView>) -> ResourceId {
        self.resources.import_image(name, image)
    }

    pub fn declare_virtual(&mut self, name: &str) -> ResourceId {
        let desc = ResourceDesc {
            name: name.to_string(),
            kind: ResourceKind::Image {
                format: vulkano::format::Format::UNDEFINED,
                extent: [0, 0],
                usage: 0,
                mip_levels: 0,
                samples: 0,
            },
        };
        self.resources.insert(desc, None)
    }

    /// Declare a pass without execution (ordering/culling only). Executing a
    /// graph containing such a pass fails loudly — use `add_pass_with` for
    /// anything that records.
    pub fn add_pass<F>(&mut self, name: &str, setup_fn: F) -> PassIndex
    where
        F: FnOnce(&mut PassBuilder),
    {
        let index = self.passes.len();
        self.passes.push(PassNode::new(name));
        self.executors.push(None);
        let mut builder = PassBuilder::new(self, index);
        setup_fn(&mut builder);
        PassIndex(index)
    }

    /// Declare a pass with its execution attached — the graph owns dispatch,
    /// there is no name-based match to fall out of. `setup_fn` declares the
    /// resource reads/writes (ordering/culling); `exec` records the pass into
    /// the frame's command buffer and should `mark_read`/`mark_write` what it
    /// actually binds so debug builds can cross-check the declaration.
    pub fn add_pass_with<F, E>(&mut self, name: &str, setup_fn: F, exec: E) -> PassIndex
    where
        F: FnOnce(&mut PassBuilder),
        E: FnMut(&mut PassContext<'_>) -> Result<(), Box<dyn std::error::Error>> + 'exec,
    {
        let index = self.add_pass(name, setup_fn);
        self.executors[index.0] = Some(Box::new(exec));
        index
    }

    pub fn mark_output(&mut self, resource: ResourceId) {
        self.output_resources.insert(resource);
    }

    pub fn enable_culling(&mut self) {
        self.culling_enabled = true;
    }

    pub fn compile(&mut self) -> Result<(), GraphError> {
        let order = self.topological_sort()?;

        if self.culling_enabled {
            self.compiled_order = self.cull_passes(&order);
        } else {
            self.compiled_order = order;
        }

        Ok(())
    }

    pub fn compile_with_pool<F>(
        &mut self,
        pool: &mut super::resource_pool::TransientResourcePool,
        mut create_fn: F,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        F: FnMut(&ResourceDesc) -> Result<Arc<ImageView>, Box<dyn std::error::Error>>,
    {
        for &id in &self.transient_resources {
            if let Some(desc) = self.resources.desc(id).cloned() {
                let image = pool.allocate(&desc, &mut create_fn)?;
                self.resources.set_image(id, image);
            }
        }

        let order = self.topological_sort()?;

        if self.culling_enabled {
            self.compiled_order = self.cull_passes(&order);
        } else {
            self.compiled_order = order;
        }

        Ok(())
    }

    pub fn compiled_order(&self) -> &[PassIndex] {
        &self.compiled_order
    }

    pub fn pass_name(&self, index: PassIndex) -> &str {
        &self.passes[index.0].name
    }

    /// Run the compiled pass order, dispatching each pass's attached
    /// executor. Loud failure modes (never a silent no-op):
    /// - a compiled pass with no executor attached → error naming the pass;
    /// - an executor returning an error → wrapped with the pass name;
    /// - (debug builds) an executor that marked a read/write it did not
    ///   declare in setup → error naming pass and resource.
    pub fn execute(
        &mut self,
        builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let executors = &mut self.executors;
        let passes = &self.passes;
        let resources = &self.resources;

        for &PassIndex(idx) in &self.compiled_order {
            let name = passes[idx].name.as_str();
            let exec = executors[idx].as_mut().ok_or_else(|| {
                format!(
                    "render graph: pass '{name}' has no executor attached \
                     (declared with add_pass instead of add_pass_with)"
                )
            })?;

            #[cfg_attr(not(debug_assertions), allow(unused_mut))]
            let mut ctx = PassContext::new(builder, resources);
            exec(&mut ctx).map_err(|e| format!("render graph: pass '{name}' failed: {e}"))?;

            #[cfg(debug_assertions)]
            Self::check_touches(&passes[idx], &ctx, resources)?;
        }
        Ok(())
    }

    /// Debug cross-check: everything the executor claimed to touch must have
    /// been declared in setup (reads against reads∪modifies, writes against
    /// writes∪modifies). Undeclared access means the topological sort never
    /// saw the dependency — ordering is luck. Returns an error (instead of
    /// panicking) so the render thread can surface it without dying.
    #[cfg(debug_assertions)]
    fn check_touches(
        pass: &PassNode,
        ctx: &PassContext<'_>,
        resources: &ResourceTable,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let res_name = |id| {
            resources
                .desc(id)
                .map(|d| d.name.as_str())
                .unwrap_or("<unknown>")
        };
        for &id in &ctx.touched_reads {
            if !pass.reads.contains(&id) && !pass.modifies.contains(&id) {
                return Err(format!(
                    "render graph: pass '{}' reads resource '{}' without declaring it \
                     (add b.read(..) in its setup)",
                    pass.name,
                    res_name(id)
                )
                .into());
            }
        }
        for &id in &ctx.touched_writes {
            if !pass.writes.contains(&id) && !pass.modifies.contains(&id) {
                return Err(format!(
                    "render graph: pass '{}' writes resource '{}' without declaring it \
                     (add b.write(..) in its setup)",
                    pass.name,
                    res_name(id)
                )
                .into());
            }
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.passes.clear();
        self.executors.clear();
        self.resources.clear();
        self.transient_resources.clear();
        self.compiled_order.clear();
        self.output_resources.clear();
    }

    fn topological_sort(&self) -> Result<Vec<PassIndex>, GraphError> {
        let n = self.passes.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        // Build adjacency: for each resource, track which pass writes/modifies it and which reads/modifies it
        // A pass that writes a resource must come before a pass that reads it
        // A pass that modifies a resource must come after the writer and before subsequent readers
        // Multiple modifies on the same resource maintain insertion order

        let mut edges: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        let mut in_degree: Vec<usize> = vec![0; n];

        // For each resource, track the chain of producers (write then modifies in order)
        let mut resource_last_writer: HashMap<ResourceId, usize> = HashMap::new();

        // First, record all writes
        for (i, pass) in self.passes.iter().enumerate() {
            for &res in &pass.writes {
                resource_last_writer.insert(res, i);
            }
        }

        // Process modifies in insertion order — each modifier depends on the last writer/modifier
        for (i, pass) in self.passes.iter().enumerate() {
            for &res in &pass.modifies {
                if let Some(&prev) = resource_last_writer.get(&res) {
                    if prev != i && edges[prev].insert(i) {
                        in_degree[i] += 1;
                    }
                }
                resource_last_writer.insert(res, i);
            }
        }

        // Process reads — each reader depends on the last writer/modifier of the resource
        for (i, pass) in self.passes.iter().enumerate() {
            for &res in &pass.reads {
                if let Some(&writer) = resource_last_writer.get(&res) {
                    if writer != i && edges[writer].insert(i) {
                        in_degree[i] += 1;
                    }
                }
            }
        }

        // Kahn's algorithm with insertion-order tiebreak
        let mut queue: Vec<usize> = Vec::new();
        for (i, &deg) in in_degree.iter().enumerate() {
            if deg == 0 {
                queue.push(i);
            }
        }

        let mut result = Vec::with_capacity(n);
        let mut head = 0;

        while head < queue.len() {
            let node = queue[head];
            head += 1;
            result.push(PassIndex(node));

            // Process neighbors in sorted order for deterministic insertion-order tiebreak
            let mut neighbors: Vec<usize> = edges[node].iter().copied().collect();
            neighbors.sort_unstable();

            for &next in &neighbors {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push(next);
                }
            }
        }

        if result.len() != n {
            return Err(GraphError::CycleDetected);
        }

        Ok(result)
    }

    fn cull_passes(&self, order: &[PassIndex]) -> Vec<PassIndex> {
        // Walk backwards: a pass is live if it writes/modifies an output resource
        // or writes/modifies a resource that a later live pass reads
        let mut live_resources: HashSet<ResourceId> = self.output_resources.clone();
        let mut live_passes: HashSet<usize> = HashSet::new();

        for &PassIndex(idx) in order.iter().rev() {
            let pass = &self.passes[idx];

            let is_live = pass.writes.iter().any(|r| live_resources.contains(r))
                || pass.modifies.iter().any(|r| live_resources.contains(r));

            if is_live {
                live_passes.insert(idx);
                for &r in &pass.reads {
                    live_resources.insert(r);
                }
                for &r in &pass.modifies {
                    live_resources.insert(r);
                }
            }
        }

        order
            .iter()
            .filter(|p| live_passes.contains(&p.0))
            .copied()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rendering::graph::resource::{ResourceDesc, ResourceKind};
    use vulkano::format::Format;

    fn dummy_desc(name: &str) -> ResourceDesc {
        ResourceDesc {
            name: name.to_string(),
            kind: ResourceKind::Image {
                format: Format::R8G8B8A8_UNORM,
                extent: [1, 1],
                usage: 0,
                mip_levels: 1,
                samples: 1,
            },
        }
    }

    fn make_graph_with_virtual_resources(count: usize) -> (RenderGraph<'static>, Vec<ResourceId>) {
        let mut graph = RenderGraph::new();
        let mut ids = Vec::new();
        for i in 0..count {
            let desc = dummy_desc(&format!("r{}", i));
            let id = graph.resources.insert(desc, None);
            ids.push(id);
        }
        (graph, ids)
    }

    #[test]
    fn test_topological_sort_linear() {
        // A writes R1, B reads R1 writes R2, C reads R2 → order is A, B, C
        let (mut graph, ids) = make_graph_with_virtual_resources(2);
        let r1 = ids[0];
        let r2 = ids[1];

        graph.add_pass("A", |b| {
            b.write(r1);
        });
        graph.add_pass("B", |b| {
            b.read(r1);
            b.write(r2);
        });
        graph.add_pass("C", |b| {
            b.read(r2);
        });

        graph.compile().unwrap();
        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_topological_sort_diamond() {
        // A writes R1+R2, B reads R1 writes R3, C reads R2 writes R4, D reads R3+R4
        let (mut graph, ids) = make_graph_with_virtual_resources(4);
        let r1 = ids[0];
        let r2 = ids[1];
        let r3 = ids[2];
        let r4 = ids[3];

        graph.add_pass("A", |b| {
            b.write(r1);
            b.write(r2);
        });
        graph.add_pass("B", |b| {
            b.read(r1);
            b.write(r3);
        });
        graph.add_pass("C", |b| {
            b.read(r2);
            b.write(r4);
        });
        graph.add_pass("D", |b| {
            b.read(r3);
            b.read(r4);
        });

        graph.compile().unwrap();
        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names[0], "A");
        assert_eq!(names[3], "D");
        // B and C can be in either order, but insertion order tiebreak means B before C
        assert_eq!(names[1], "B");
        assert_eq!(names[2], "C");
    }

    #[test]
    fn test_cycle_detection() {
        // A reads R1 writes R2, B reads R2 writes R1 → cycle
        let (mut graph, ids) = make_graph_with_virtual_resources(2);
        let r1 = ids[0];
        let r2 = ids[1];

        graph.add_pass("A", |b| {
            b.read(r1);
            b.write(r2);
        });
        graph.add_pass("B", |b| {
            b.read(r2);
            b.write(r1);
        });

        let result = graph.compile();
        assert!(result.is_err());
    }

    #[test]
    fn test_insertion_order_tiebreak() {
        // Two passes with no shared resources → maintain insertion order
        let mut graph = RenderGraph::new();

        graph.add_pass("first", |_| {});
        graph.add_pass("second", |_| {});

        graph.compile().unwrap();
        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn test_modify_ordering() {
        // A writes target, B modifies target, C modifies target → order is A, B, C
        let (mut graph, ids) = make_graph_with_virtual_resources(1);
        let target = ids[0];

        graph.add_pass("A", |b| {
            b.write(target);
        });
        graph.add_pass("B", |b| {
            b.modify(target);
        });
        graph.add_pass("C", |b| {
            b.modify(target);
        });

        graph.compile().unwrap();
        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names, vec!["A", "B", "C"]);
    }

    #[test]
    fn test_culling_removes_dead_pass() {
        let (mut graph, ids) = make_graph_with_virtual_resources(3);
        let r1 = ids[0];
        let r2 = ids[1];
        let r3 = ids[2]; // nobody reads this

        graph.add_pass("geometry", |b| {
            b.write(r1);
        });
        graph.add_pass("lighting", |b| {
            b.read(r1);
            b.write(r2);
        });
        graph.add_pass("dummy", |b| {
            b.write(r3);
        });

        graph.mark_output(r2);
        graph.enable_culling();
        graph.compile().unwrap();

        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names, vec!["geometry", "lighting"]);
    }

    #[test]
    fn test_culling_keeps_modify_chain() {
        let (mut graph, ids) = make_graph_with_virtual_resources(2);
        let gbuffer = ids[0];
        let target = ids[1];

        graph.add_pass("geometry", |b| {
            b.write(gbuffer);
        });
        graph.add_pass("lighting", |b| {
            b.read(gbuffer);
            b.write(target);
        });
        graph.add_pass("grid", |b| {
            b.read(gbuffer);
            b.modify(target);
        });
        graph.add_pass("debug_draw", |b| {
            b.read(gbuffer);
            b.modify(target);
        });

        graph.mark_output(target);
        graph.enable_culling();
        graph.compile().unwrap();

        let names: Vec<&str> = graph
            .compiled_order()
            .iter()
            .map(|&p| graph.pass_name(p))
            .collect();
        assert_eq!(names, vec!["geometry", "lighting", "grid", "debug_draw"]);
    }
}
