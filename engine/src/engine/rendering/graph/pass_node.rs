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

pub struct PassBuilder<'a> {
    graph: &'a mut RenderGraph,
    pass_index: usize,
}

impl<'a> PassBuilder<'a> {
    pub(crate) fn new(graph: &'a mut RenderGraph, pass_index: usize) -> Self {
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

pub struct PassContext<'a> {
    pub builder: &'a mut AutoCommandBufferBuilder<vulkano::command_buffer::PrimaryAutoCommandBuffer>,
    pub resources: &'a ResourceTable,
}

impl<'a> PassContext<'a> {
    pub fn get_image(&self, id: ResourceId) -> Option<&Arc<ImageView>> {
        self.resources.get_image(id)
    }
}
