use super::render_graph::RenderGraph;
use super::resource::{ResourceDesc, ResourceId, ResourceKind, ResourceTable};
use std::sync::Arc;
use vulkano::command_buffer::AutoCommandBufferBuilder;
use vulkano::image::view::ImageView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PassIndex(pub(crate) usize);

pub struct PassNode {
    pub name: String,
    pub reads: Vec<ResourceId>,
    pub writes: Vec<ResourceId>,
    pub modifies: Vec<ResourceId>,
}

impl PassNode {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            reads: Vec::new(),
            writes: Vec::new(),
            modifies: Vec::new(),
        }
    }
}

pub struct PassBuilder<'a, 'exec> {
    graph: &'a mut RenderGraph<'exec>,
    pass_index: usize,
}

impl<'a, 'exec> PassBuilder<'a, 'exec> {
    pub(crate) fn new(graph: &'a mut RenderGraph<'exec>, pass_index: usize) -> Self {
        Self { graph, pass_index }
    }

    pub fn read(&mut self, resource: ResourceId) {
        self.graph.passes[self.pass_index].reads.push(resource);
    }

    pub fn write(&mut self, resource: ResourceId) {
        self.graph.passes[self.pass_index].writes.push(resource);
    }

    pub fn modify(&mut self, resource: ResourceId) {
        self.graph.passes[self.pass_index].modifies.push(resource);
    }

    pub fn create_transient(&mut self, name: &str, kind: ResourceKind) -> ResourceId {
        let desc = ResourceDesc {
            name: name.to_string(),
            kind,
        };
        let id = self.graph.resources.insert(desc, None);
        self.graph.transient_resources.push(id);
        id
    }
}

/// Per-pass execution context handed to executors by [`RenderGraph::execute`].
///
/// In debug builds it also records what the executor claims to touch
/// (`mark_read`/`mark_write`); after the executor returns, the graph checks
/// those claims against the pass's declared reads/writes and fails loudly on
/// undeclared access (undeclared = the topological sort never saw the
/// dependency, so ordering is luck).
pub struct PassContext<'a> {
    pub builder:
        &'a mut AutoCommandBufferBuilder<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    pub resources: &'a ResourceTable,
    #[cfg(debug_assertions)]
    pub(crate) touched_reads: Vec<ResourceId>,
    #[cfg(debug_assertions)]
    pub(crate) touched_writes: Vec<ResourceId>,
}

impl<'a> PassContext<'a> {
    pub(crate) fn new(
        builder: &'a mut AutoCommandBufferBuilder<
            vulkano::command_buffer::PrimaryAutoCommandBuffer,
        >,
        resources: &'a ResourceTable,
    ) -> Self {
        Self {
            builder,
            resources,
            #[cfg(debug_assertions)]
            touched_reads: Vec::new(),
            #[cfg(debug_assertions)]
            touched_writes: Vec::new(),
        }
    }

    /// Fetch a resource image; counts as a read of `id` for the debug
    /// declaration check.
    pub fn get_image(&mut self, id: ResourceId) -> Option<&Arc<ImageView>> {
        self.mark_read(id);
        self.resources.get_image(id)
    }

    /// Record that this pass actually reads `id` (e.g. samples it through a
    /// descriptor set it binds). Debug builds verify the pass declared the
    /// read (or a modify) in its setup. No-op in release.
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub fn mark_read(&mut self, id: ResourceId) {
        #[cfg(debug_assertions)]
        self.touched_reads.push(id);
    }

    /// Record that this pass actually writes `id` (renders/dispatches into
    /// it). Debug builds verify the pass declared the write (or a modify) in
    /// its setup. No-op in release.
    #[cfg_attr(not(debug_assertions), allow(unused_variables))]
    pub fn mark_write(&mut self, id: ResourceId) {
        #[cfg(debug_assertions)]
        self.touched_writes.push(id);
    }
}
