use crate::{AnimatedSprite, Sprite};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
fn generation() -> u64 {
    NEXT_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .expect("scene handle generations exhausted")
}

pub struct Handle<T> {
    index: usize,
    generation: u64,
    marker: PhantomData<fn() -> T>,
}
impl<T> Copy for Handle<T> {}
impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T> Eq for Handle<T> {}
impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&(self.index, self.generation), state);
    }
}
impl<T> std::fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handle")
            .field("index", &self.index)
            .field("generation", &self.generation)
            .finish()
    }
}
struct Slot<T> {
    generation: u64,
    value: Option<T>,
    next: Option<usize>,
}
struct Arena<T> {
    slots: Vec<Slot<T>>,
    free: Option<usize>,
}
impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: None,
        }
    }
}
impl<T> Arena<T> {
    fn add(&mut self, value: T) -> Handle<T> {
        let generation = generation();
        let slot = Slot {
            generation,
            value: Some(value),
            next: None,
        };
        let index = if let Some(index) = self.free {
            self.free = self.slots[index].next;
            self.slots[index] = slot;
            index
        } else {
            let index = self.slots.len();
            self.slots.push(slot);
            index
        };
        Handle {
            index,
            generation,
            marker: PhantomData,
        }
    }
    fn get(&self, handle: Handle<T>) -> Option<&T> {
        let slot = self.slots.get(handle.index)?;
        (slot.generation == handle.generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }
    fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let slot = self.slots.get_mut(handle.index)?;
        (slot.generation == handle.generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }
    fn remove(&mut self, handle: Handle<T>) -> bool {
        let Some(slot) = self.slots.get_mut(handle.index) else {
            return false;
        };
        if slot.generation != handle.generation || slot.value.is_none() {
            return false;
        }
        slot.value = None;
        slot.next = self.free;
        self.free = Some(handle.index);
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawHandle {
    Sprite(Handle<Sprite>),
    Animated(Handle<AnimatedSprite>),
}

#[derive(Default)]
pub struct Scene {
    sprites: Arena<Sprite>,
    animated: Arena<AnimatedSprite>,
    pub(crate) order: Vec<DrawHandle>,
}
mod sealed {
    pub trait Sealed {}
}
pub trait SceneObject: sealed::Sealed + Sized {
    #[doc(hidden)]
    fn insert(self, scene: &mut Scene) -> Handle<Self>;
    #[doc(hidden)]
    fn resolve(scene: &Scene, handle: Handle<Self>) -> Option<&Self>;
    #[doc(hidden)]
    fn resolve_mut(scene: &mut Scene, handle: Handle<Self>) -> Option<&mut Self>;
    #[doc(hidden)]
    fn erase(scene: &mut Scene, handle: Handle<Self>) -> bool;
}
macro_rules! scene_object {
    ($ty:ty, $arena:ident, $kind:ident) => {
        impl sealed::Sealed for $ty {}
        impl SceneObject for $ty {
            fn insert(self, scene: &mut Scene) -> Handle<Self> {
                let handle = scene.$arena.add(self);
                scene.order.push(DrawHandle::$kind(handle));
                handle
            }
            fn resolve(scene: &Scene, handle: Handle<Self>) -> Option<&Self> {
                scene.$arena.get(handle)
            }
            fn resolve_mut(scene: &mut Scene, handle: Handle<Self>) -> Option<&mut Self> {
                scene.$arena.get_mut(handle)
            }
            fn erase(scene: &mut Scene, handle: Handle<Self>) -> bool {
                if !scene.$arena.remove(handle) {
                    return false;
                }
                scene.order.retain(|h| *h != DrawHandle::$kind(handle));
                true
            }
        }
    };
}
scene_object!(Sprite, sprites, Sprite);
scene_object!(AnimatedSprite, animated, Animated);
impl Scene {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add<T: SceneObject>(&mut self, object: T) -> Handle<T> {
        object.insert(self)
    }
    pub fn get<T: SceneObject>(&self, handle: Handle<T>) -> Option<&T> {
        T::resolve(self, handle)
    }
    pub fn get_mut<T: SceneObject>(&mut self, handle: Handle<T>) -> Option<&mut T> {
        T::resolve_mut(self, handle)
    }
    pub fn remove<T: SceneObject>(&mut self, handle: Handle<T>) -> bool {
        T::erase(self, handle)
    }
    pub fn clear(&mut self) {
        *self = Self::new();
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn update_animations(&mut self, dt: f32) {
        for handle in &self.order {
            if let DrawHandle::Animated(handle) = *handle {
                self.animated.get_mut(handle).unwrap().update(dt);
            }
        }
    }
    pub(crate) fn sync(&mut self) {
        for handle in &self.order {
            if let DrawHandle::Animated(handle) = *handle {
                self.animated.get_mut(handle).unwrap().sync();
            }
        }
    }
}
impl<T: SceneObject> std::ops::Index<Handle<T>> for Scene {
    type Output = T;
    fn index(&self, handle: Handle<T>) -> &T {
        self.get(handle).expect("invalid scene handle")
    }
}
impl<T: SceneObject> std::ops::IndexMut<Handle<T>> for Scene {
    fn index_mut(&mut self, handle: Handle<T>) -> &mut T {
        self.get_mut(handle).expect("invalid scene handle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpriteFrames, Texture};
    use std::sync::Arc;
    fn texture() -> Arc<Texture> {
        Arc::new(Texture::from_rgba(1, 1, vec![255; 4]).unwrap())
    }

    #[test]
    fn stale_handles_cannot_alias_reused_slots_or_other_scenes() {
        let mut scene = Scene::new();
        let old = scene.add(Sprite::new(texture()));
        assert!(scene.remove(old));
        assert!(!scene.remove(old));
        let new = scene.add(Sprite::new(texture()));
        assert_eq!(old.index, new.index);
        assert_ne!(old, new);
        assert!(scene.get(old).is_none());
        assert!(scene.get_mut(old).is_none());
        let mut other = Scene::new();
        let foreign = other.add(Sprite::new(texture()));
        assert!(scene.get(foreign).is_none());
        assert!(!scene.remove(foreign));
        scene.clear();
        let after_clear = scene.add(Sprite::new(texture()));
        assert!(scene.get(new).is_none());
        assert_ne!(after_clear, new);
        drop(scene);
        let mut replacement = Scene::new();
        replacement.add(Sprite::new(texture()));
        assert!(replacement.get(after_clear).is_none());
    }

    #[test]
    fn mixed_order_survives_removal_and_slot_reuse() {
        let mut scene = Scene::new();
        let a = scene.add(Sprite::new(texture()));
        let b = scene.add(AnimatedSprite::new(Arc::new(SpriteFrames::new(texture()))));
        let c = scene.add(Sprite::new(texture()));
        assert_eq!(
            scene.order,
            [
                DrawHandle::Sprite(a),
                DrawHandle::Animated(b),
                DrawHandle::Sprite(c)
            ]
        );
        scene.remove(a);
        let d = scene.add(Sprite::new(texture()));
        assert_eq!(d.index, a.index);
        assert_eq!(
            scene.order,
            [
                DrawHandle::Animated(b),
                DrawHandle::Sprite(c),
                DrawHandle::Sprite(d)
            ]
        );
        scene[b].position.x = 12.;
        assert_eq!(scene.get(b).unwrap().position.x, 12.);
        scene.remove(b);
        let e = scene.add(AnimatedSprite::new(Arc::new(SpriteFrames::new(texture()))));
        assert_eq!(b.index, e.index);
        assert!(scene.get(b).is_none());
        assert_eq!(scene.order.last(), Some(&DrawHandle::Animated(e)));
    }

    #[test]
    fn handles_do_not_keep_scene_objects_alive() {
        let texture = texture();
        let weak = Arc::downgrade(&texture);
        let mut scene = Scene::new();
        let sprite = scene.add(Sprite::new(texture.clone()));
        let animated = scene.add(AnimatedSprite::new(Arc::new(SpriteFrames::new(texture))));
        assert_eq!(weak.strong_count(), 3);
        scene.remove(sprite);
        assert_eq!(weak.strong_count(), 2);
        drop(scene);
        assert!(weak.upgrade().is_none());
        let mut replacement = Scene::new();
        replacement.add(AnimatedSprite::new(Arc::new(SpriteFrames::new(
            self::texture(),
        ))));
        assert!(replacement.get(animated).is_none());
    }
}
