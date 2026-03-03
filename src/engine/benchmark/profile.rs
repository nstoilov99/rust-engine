use parking_lot::Mutex;
use puffin::{FrameData, ScopeCollection, ScopeId};
use std::collections::HashMap;
use std::sync::Arc;

const CATEGORY_SCOPES: &[(&str, &[&str])] = &[
    ("frame_update", &["frame_update"]),
    ("frame_render", &["frame_render"]),
    ("ecs_systems", &["ecs_systems"]),
    ("transform_propagation", &["transform_propagation"]),
    ("physics", &["physics_step"]),
    ("prepare_mesh_data", &["prepare_mesh_data"]),
    ("prepare_light_data", &["prepare_light_data"]),
    ("swapchain_acquire", &["acquire_swapchain_image"]),
    ("command_buffer_setup", &["command_buffer_setup"]),
    ("command_buffer_build", &["command_buffer_build"]),
    ("geometry_pass", &["geometry_pass"]),
    ("lighting_pass", &["lighting_pass"]),
    ("grid_pass", &["grid_pass"]),
    ("swapchain_present", &["swapchain_present"]),
];

pub struct BenchmarkProfileCollector {
    scope_collection: ScopeCollection,
    string_cache: HashMap<ScopeId, Arc<str>>,
    totals_ms: HashMap<String, f64>,
    sampled_frames: u32,
    sampling_enabled: bool,
}

impl BenchmarkProfileCollector {
    fn new() -> Self {
        Self {
            scope_collection: ScopeCollection::default(),
            string_cache: HashMap::new(),
            totals_ms: HashMap::new(),
            sampled_frames: 0,
            sampling_enabled: false,
        }
    }

    pub fn start_sampling(&mut self) {
        self.totals_ms.clear();
        self.sampled_frames = 0;
        self.sampling_enabled = true;
    }

    pub fn category_averages(&self) -> HashMap<String, f64> {
        if self.sampled_frames == 0 {
            return HashMap::new();
        }

        self.totals_ms
            .iter()
            .map(|(name, total_ms)| (name.clone(), total_ms / self.sampled_frames as f64))
            .collect()
    }

    fn process_frame(&mut self, frame_data: &Arc<FrameData>) {
        for scope_detail in &frame_data.scope_delta {
            self.scope_collection.insert(scope_detail.clone());
        }

        if !self.sampling_enabled {
            return;
        }

        #[allow(irrefutable_let_patterns)]
        let Ok(unpacked) = frame_data.unpacked() else {
            return;
        };

        let mut frame_totals = HashMap::<&'static str, f64>::new();
        for (_, stream_info) in &unpacked.thread_streams {
            let top_scopes = match puffin::Reader::from_start(&stream_info.stream).read_top_scopes()
            {
                Ok(scopes) => scopes,
                Err(_) => continue,
            };
            self.collect_scope_totals(&top_scopes, &stream_info.stream, &mut frame_totals);
        }

        self.sampled_frames += 1;
        for (category, value_ms) in frame_totals {
            *self.totals_ms.entry(category.to_string()).or_default() += value_ms;
        }
    }

    fn collect_scope_totals(
        &mut self,
        scopes: &[puffin::Scope<'_>],
        stream: &puffin::Stream,
        frame_totals: &mut HashMap<&'static str, f64>,
    ) {
        for scope in scopes {
            let scope_name = self.scope_name(&scope.id);
            for (category, scope_names) in CATEGORY_SCOPES {
                if scope_names
                    .iter()
                    .any(|candidate| scope_name.as_ref() == *candidate)
                {
                    *frame_totals.entry(*category).or_default() +=
                        scope.record.duration_ns as f64 / 1_000_000.0;
                    break;
                }
            }

            if scope.child_begin_position < scope.child_end_position {
                if let Ok(reader) = puffin::Reader::with_offset(stream, scope.child_begin_position)
                {
                    let child_scopes: Vec<_> = reader
                        .take_while(|result| {
                            result
                                .as_ref()
                                .map(|child| child.record.start_ns < scope.record.stop_ns())
                                .unwrap_or(false)
                        })
                        .filter_map(Result::ok)
                        .collect();
                    self.collect_scope_totals(&child_scopes, stream, frame_totals);
                }
            }
        }
    }

    fn scope_name(&mut self, scope_id: &ScopeId) -> Arc<str> {
        if let Some(name) = self.string_cache.get(scope_id) {
            return name.clone();
        }

        let name: Arc<str> = if let Some(details) = self.scope_collection.fetch_by_id(scope_id) {
            Arc::from(
                details
                    .scope_name
                    .as_ref()
                    .map(|value| value.as_ref())
                    .unwrap_or_else(|| details.function_name.as_ref()),
            )
        } else {
            Arc::from(format!("scope_{}", scope_id.0))
        };

        self.string_cache.insert(*scope_id, name.clone());
        name
    }
}

pub fn create_profile_sink() -> (Arc<Mutex<BenchmarkProfileCollector>>, puffin::FrameSinkId) {
    let collector = Arc::new(Mutex::new(BenchmarkProfileCollector::new()));
    let sink_collector = collector.clone();

    let sink_id = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame_data| {
        sink_collector.lock().process_frame(&frame_data);
    }));

    (collector, sink_id)
}
