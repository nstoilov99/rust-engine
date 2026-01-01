//! Puffin frame data collector
//!
//! Extracts frame data from puffin's GlobalProfiler and converts it
//! to our internal ProfileFrame format.

use super::data::{ProfileFrame, ProfileScope, ProfileThread};
use puffin::{FrameData, ScopeCollection, ScopeId};
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Cached scope strings to avoid per-frame allocations
/// Scope IDs are stable across frames, so we cache the computed name and location
type ScopeStringCache = HashMap<ScopeId, (Arc<str>, Arc<str>)>;

/// Collector that receives frame data from puffin
pub struct ProfilerCollector {
    /// Sender to the profiler panel
    tx: Sender<Arc<ProfileFrame>>,
    /// Frame counter (puffin doesn't always provide reliable frame numbers)
    frame_count: u64,
    /// Scope collection to look up scope details
    scope_collection: ScopeCollection,
    /// Cache of (name, location) strings per scope ID to avoid repeated allocations
    string_cache: ScopeStringCache,
}

impl ProfilerCollector {
    /// Create a new collector with the given sender
    pub fn new(tx: Sender<Arc<ProfileFrame>>) -> Self {
        Self {
            tx,
            frame_count: 0,
            scope_collection: ScopeCollection::default(),
            string_cache: HashMap::with_capacity(256), // Pre-allocate for typical scope count
        }
    }

    /// Process a puffin frame and send it to the UI
    pub fn process_frame(&mut self, frame_data: &Arc<FrameData>) {
        self.frame_count += 1;

        // Prevent unbounded memory growth: clear caches periodically
        // The scope_collection is just a lookup cache that gets rebuilt from scope_delta
        // Typical apps have <500 unique scopes, so 1000 is a generous limit
        if self.frame_count % 1000 == 0 {
            let scope_count = self.scope_collection.scopes_by_id().len();
            if scope_count > 1000 {
                self.scope_collection = ScopeCollection::default();
                self.string_cache.clear();
            }
        }

        // Update scope collection with new scope details from this frame
        for scope_detail in &frame_data.scope_delta {
            self.scope_collection.insert(scope_detail.clone());
        }

        // Unpack the frame data
        let Ok(unpacked) = frame_data.unpacked() else {
            return;
        };

        let mut threads = Vec::new();
        let mut total_scopes = 0;

        for (thread_info, stream_info) in &unpacked.thread_streams {
            let scopes = self.parse_scopes(&stream_info.stream);
            let scope_count: usize = scopes.iter().map(|s| count_scopes(s)).sum();
            total_scopes += scope_count;

            let max_depth = calculate_max_depth(&scopes);

            threads.push(ProfileThread {
                name: thread_info.name.clone(),
                scopes,
                max_depth,
            });
        }

        let profile_frame = ProfileFrame {
            frame_number: self.frame_count,
            duration_ns: frame_data.duration_ns(),
            threads,
            total_scopes,
            data_size_bytes: frame_data.bytes_of_ram_used(),
        };

        // Send to UI, ignore errors (receiver might be dropped)
        let _ = self.tx.send(Arc::new(profile_frame));
    }

    /// Parse puffin stream into our scope format
    fn parse_scopes(&mut self, stream: &puffin::Stream) -> Vec<ProfileScope> {
        let top_scopes = match puffin::Reader::from_start(stream).read_top_scopes() {
            Ok(scopes) => scopes,
            Err(_) => return Vec::new(),
        };

        self.convert_scopes(&top_scopes, stream, 0)
    }

    /// Convert puffin Scope objects to our ProfileScope format recursively
    fn convert_scopes(
        &mut self,
        scopes: &[puffin::Scope<'_>],
        stream: &puffin::Stream,
        depth: usize,
    ) -> Vec<ProfileScope> {
        let mut result = Vec::with_capacity(scopes.len());

        for scope in scopes {
            // Look up cached strings or compute and cache them
            let (name, location) = self.get_or_cache_scope_strings(&scope.id);

            // Parse children
            let children = if scope.child_begin_position < scope.child_end_position {
                match puffin::Reader::with_offset(stream, scope.child_begin_position) {
                    Ok(reader) => {
                        let child_scopes: Vec<_> = reader
                            .take_while(|result| {
                                result
                                    .as_ref()
                                    .map(|s| s.record.start_ns < scope.record.stop_ns())
                                    .unwrap_or(false)
                            })
                            .filter_map(|r| r.ok())
                            .collect();
                        self.convert_scopes(&child_scopes, stream, depth + 1)
                    }
                    Err(_) => Vec::new(),
                }
            } else {
                Vec::new()
            };

            result.push(ProfileScope {
                id: format!("{}", scope.id.0),
                name,     // Pass Arc<str> directly - no allocation
                location, // Pass Arc<str> directly - no allocation
                start_ns: scope.record.start_ns,
                duration_ns: scope.record.duration_ns,
                depth,
                children,
            });
        }

        result
    }

    /// Get or compute and cache scope name and location strings
    /// This avoids repeated string allocations for the same scope ID across frames
    fn get_or_cache_scope_strings(&mut self, scope_id: &ScopeId) -> (Arc<str>, Arc<str>) {
        // Check cache first (fast path)
        if let Some(cached) = self.string_cache.get(scope_id) {
            return (cached.0.clone(), cached.1.clone());
        }

        // Compute strings (slow path - only happens once per unique scope)
        let (name, location): (Arc<str>, Arc<str>) = if let Some(details) = self.scope_collection.fetch_by_id(scope_id) {
            let name_str = details
                .scope_name
                .as_ref()
                .map(|s| s.as_ref())
                .unwrap_or_else(|| details.function_name.as_ref());
            let location = format!("{}:{}", details.file_path, details.line_nr);
            (Arc::from(name_str), Arc::from(location.as_str()))
        } else {
            // Fallback if details not found
            let name = format!("scope_{}", scope_id.0);
            (Arc::from(name.as_str()), Arc::from(""))
        };

        // Cache for future use
        self.string_cache.insert(*scope_id, (name.clone(), location.clone()));
        (name, location)
    }
}

/// Create a channel and register a frame sink with puffin
pub fn create_profiler_channel() -> (Receiver<Arc<ProfileFrame>>, puffin::FrameSinkId) {
    let (tx, rx) = channel();

    let collector = std::sync::Mutex::new(ProfilerCollector::new(tx));

    let sink_id = puffin::GlobalProfiler::lock().add_sink(Box::new(move |frame_data| {
        if let Ok(mut collector) = collector.lock() {
            collector.process_frame(&frame_data);
        }
    }));

    (rx, sink_id)
}

/// Count total scopes recursively
fn count_scopes(scope: &ProfileScope) -> usize {
    1 + scope.children.iter().map(|c| count_scopes(c)).sum::<usize>()
}

/// Calculate maximum depth in scope tree
fn calculate_max_depth(scopes: &[ProfileScope]) -> usize {
    fn depth_recursive(scope: &ProfileScope) -> usize {
        if scope.children.is_empty() {
            scope.depth
        } else {
            scope
                .children
                .iter()
                .map(|c| depth_recursive(c))
                .max()
                .unwrap_or(scope.depth)
        }
    }

    scopes.iter().map(|s| depth_recursive(s)).max().unwrap_or(0)
}
