use std::sync::Arc;
use vulkano::format::Format;
use vulkano::image::view::ImageView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceId(pub(crate) u32);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ResourceDesc {
    pub name: String,
    pub kind: ResourceKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    Image {
        format: Format,
        extent: [u32; 2],
        usage: u32,
        mip_levels: u32,
        samples: u32,
    },
}

#[derive(Default)]
pub struct ResourceTable {
    descs: Vec<ResourceDesc>,
    images: Vec<Option<Arc<ImageView>>>,
}

impl ResourceTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, desc: ResourceDesc, image: Option<Arc<ImageView>>) -> ResourceId {
        let id = ResourceId(self.descs.len() as u32);
        self.descs.push(desc);
        self.images.push(image);
        id
    }

    pub fn import_image(&mut self, name: &str, image: Arc<ImageView>) -> ResourceId {
        let img = image.image();
        let extent = img.extent();
        let desc = ResourceDesc {
            name: name.to_string(),
            kind: ResourceKind::Image {
                format: img.format(),
                extent: [extent[0], extent[1]],
                usage: 0,
                mip_levels: img.mip_levels(),
                samples: 1,
            },
        };
        self.insert(desc, Some(image))
    }

    pub fn get_image(&self, id: ResourceId) -> Option<&Arc<ImageView>> {
        self.images.get(id.0 as usize).and_then(|v| v.as_ref())
    }

    pub fn set_image(&mut self, id: ResourceId, image: Arc<ImageView>) {
        if let Some(slot) = self.images.get_mut(id.0 as usize) {
            *slot = Some(image);
        }
    }

    pub fn desc(&self, id: ResourceId) -> Option<&ResourceDesc> {
        self.descs.get(id.0 as usize)
    }

    pub fn len(&self) -> usize {
        self.descs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.descs.is_empty()
    }

    pub fn clear(&mut self) {
        self.descs.clear();
        self.images.clear();
    }
}
