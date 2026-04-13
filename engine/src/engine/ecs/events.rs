//! Double-buffered event system for decoupled communication between systems.
//!
//! Events sent in frame N are readable in frame N+1. Call `update()` (or
//! `EventStorage::update_all()`) once per frame to swap buffers.

use super::resources::PlayMode;
use hecs::Entity;
use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Marker trait for event types.
pub trait Event: Send + Sync + 'static {}

/// Double-buffered event storage for a single event type.
///
/// Events sent this frame go into `current`.
/// `iter()` reads from `previous` (events from last frame).
/// `update()` swaps the buffers (called once per frame).
pub struct Events<T: Event> {
    current: Vec<T>,
    previous: Vec<T>,
}

impl<T: Event> Events<T> {
    pub fn new() -> Self {
        Self {
            current: Vec::new(),
            previous: Vec::new(),
        }
    }

    /// Send an event (written to current buffer, readable next frame).
    pub fn send(&mut self, event: T) {
        self.current.push(event);
    }

    /// Iterate events from the previous frame.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.previous.iter()
    }

    /// Swap buffers: current becomes previous, current is cleared.
    /// Call once per frame at frame start.
    pub fn update(&mut self) {
        self.previous.clear();
        std::mem::swap(&mut self.current, &mut self.previous);
    }

    /// Clear both buffers.
    pub fn clear(&mut self) {
        self.current.clear();
        self.previous.clear();
    }

    /// Number of events available to read (from previous frame).
    pub fn len(&self) -> usize {
        self.previous.len()
    }

    /// Whether there are no events to read.
    pub fn is_empty(&self) -> bool {
        self.previous.is_empty()
    }
}

impl<T: Event> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// Wrapper for sending events.
pub struct EventWriter<'a, T: Event> {
    events: &'a mut Events<T>,
}

impl<'a, T: Event> EventWriter<'a, T> {
    pub fn new(events: &'a mut Events<T>) -> Self {
        Self { events }
    }

    pub fn send(&mut self, event: T) {
        self.events.send(event);
    }

    pub fn send_batch(&mut self, events: impl IntoIterator<Item = T>) {
        for event in events {
            self.events.send(event);
        }
    }
}

/// Wrapper for reading events.
pub struct EventReader<'a, T: Event> {
    events: &'a Events<T>,
}

impl<'a, T: Event> EventReader<'a, T> {
    pub fn new(events: &'a Events<T>) -> Self {
        Self { events }
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.events.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }
}

// === Type-erased event storage ===

/// Trait for type-erased event buffer operations.
pub trait EventBufferOps: Any + Send + Sync {
    fn update_buffer(&mut self);
    fn clear_buffer(&mut self);
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Event> EventBufferOps for Events<T> {
    fn update_buffer(&mut self) {
        self.update();
    }

    fn clear_buffer(&mut self) {
        self.clear();
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Collection of all event buffers, keyed by event TypeId.
pub struct EventStorage {
    buffers: HashMap<TypeId, Box<dyn EventBufferOps>>,
}

impl EventStorage {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Register an event type. Idempotent — does nothing if already registered.
    pub fn register<T: Event>(&mut self) {
        self.buffers
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(Events::<T>::new()));
    }

    /// Get immutable access to events of type T.
    pub fn get<T: Event>(&self) -> Option<&Events<T>> {
        self.buffers
            .get(&TypeId::of::<T>())
            .and_then(|b| b.as_any().downcast_ref::<Events<T>>())
    }

    /// Get mutable access to events of type T.
    pub fn get_mut<T: Event>(&mut self) -> Option<&mut Events<T>> {
        self.buffers
            .get_mut(&TypeId::of::<T>())
            .and_then(|b| b.as_any_mut().downcast_mut::<Events<T>>())
    }

    /// Send an event. Auto-registers the event type if not already registered.
    pub fn send<T: Event>(&mut self, event: T) {
        self.register::<T>();
        if let Some(events) = self.get_mut::<T>() {
            events.send(event);
        }
    }

    /// Update all event buffers (swap current/previous). Call once per frame.
    pub fn update_all(&mut self) {
        for buffer in self.buffers.values_mut() {
            buffer.update_buffer();
        }
    }

    /// Clear all event buffers.
    pub fn clear_all(&mut self) {
        for buffer in self.buffers.values_mut() {
            buffer.clear_buffer();
        }
    }
}

impl Default for EventStorage {
    fn default() -> Self {
        Self::new()
    }
}

// === Core Event Types ===

/// Entity spawned event.
#[derive(Debug, Clone)]
pub struct EntitySpawned {
    pub entity: Entity,
    pub name: Option<String>,
}
impl Event for EntitySpawned {}

/// Entity deleted event.
#[derive(Debug, Clone)]
pub struct EntityDeleted {
    pub entity: Entity,
}
impl Event for EntityDeleted {}

/// Selection changed in editor.
#[derive(Debug, Clone)]
pub struct SelectionChanged {
    pub previous: Option<Entity>,
    pub current: Option<Entity>,
}
impl Event for SelectionChanged {}

/// Play mode changed.
#[derive(Debug, Clone)]
pub struct PlayModeChanged {
    pub previous: PlayMode,
    pub current: PlayMode,
}
impl Event for PlayModeChanged {}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple test event type
    #[derive(Debug, Clone, PartialEq)]
    struct TestEvent(i32);
    impl Event for TestEvent {}

    #[derive(Debug, Clone, PartialEq)]
    struct OtherEvent(String);
    impl Event for OtherEvent {}

    #[test]
    fn events_not_readable_same_frame() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(1));
        // Before update(), events go to `current` — iter reads from `previous`
        assert!(events.is_empty());
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn events_readable_after_update() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(42));
        events.update(); // swap: current → previous

        assert!(!events.is_empty());
        assert_eq!(events.len(), 1);
        let collected: Vec<_> = events.iter().collect();
        assert_eq!(collected[0], &TestEvent(42));
    }

    #[test]
    fn events_cleared_after_two_updates() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(1));
        events.update(); // frame N events now readable
        assert_eq!(events.len(), 1);

        events.update(); // frame N events cleared, nothing new
        assert!(events.is_empty());
    }

    #[test]
    fn multiple_events_in_one_frame() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(1));
        events.send(TestEvent(2));
        events.send(TestEvent(3));
        events.update();

        assert_eq!(events.len(), 3);
        let values: Vec<i32> = events.iter().map(|e| e.0).collect();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn clear_removes_all_events() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(1));
        events.update();
        events.send(TestEvent(2));

        events.clear();
        assert!(events.is_empty());
        events.update();
        assert!(events.is_empty());
    }

    #[test]
    fn event_writer_sends_batch() {
        let mut events = Events::<TestEvent>::new();
        {
            let mut writer = EventWriter::new(&mut events);
            writer.send_batch(vec![TestEvent(10), TestEvent(20)]);
        }
        events.update();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn event_reader_iterates() {
        let mut events = Events::<TestEvent>::new();
        events.send(TestEvent(5));
        events.update();

        let reader = EventReader::new(&events);
        assert!(!reader.is_empty());
        assert_eq!(reader.len(), 1);
        let collected: Vec<_> = reader.iter().collect();
        assert_eq!(collected[0], &TestEvent(5));
    }

    #[test]
    fn event_storage_register_and_send() {
        let mut storage = EventStorage::new();
        storage.register::<TestEvent>();

        storage.send(TestEvent(99));
        storage.update_all();

        let events = storage
            .get::<TestEvent>()
            .expect("TestEvent should be registered");
        assert_eq!(events.len(), 1);
        assert_eq!(events.iter().next(), Some(&TestEvent(99)));
    }

    #[test]
    fn event_storage_auto_registers_on_send() {
        let mut storage = EventStorage::new();
        // Don't call register — send should auto-register
        storage.send(TestEvent(7));
        storage.update_all();

        let events = storage.get::<TestEvent>().expect("auto-registered");
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn event_storage_multiple_types() {
        let mut storage = EventStorage::new();
        storage.send(TestEvent(1));
        storage.send(OtherEvent("hello".into()));
        storage.update_all();

        assert_eq!(storage.get::<TestEvent>().expect("registered").len(), 1);
        assert_eq!(storage.get::<OtherEvent>().expect("registered").len(), 1);
    }

    #[test]
    fn event_storage_clear_all() {
        let mut storage = EventStorage::new();
        storage.send(TestEvent(1));
        storage.update_all();
        assert_eq!(storage.get::<TestEvent>().expect("registered").len(), 1);

        storage.clear_all();
        assert_eq!(
            storage.get::<TestEvent>().expect("still registered").len(),
            0
        );
    }
}
