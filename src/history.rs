//! Undo/redo.
//!
//! Every edit is recorded as the [`Action`] that reverts it. Applying an
//! action returns its inverse, which goes onto the opposite stack.

use crate::canvas::{Layer, LayerId, Rgba, TileKey};

pub enum Action {
    /// Puts these tiles back into the layer (`None` = tile didn't exist),
    /// and the layer's default color if it changed.
    Tiles {
        layer_id: LayerId,
        tiles: Vec<(TileKey, Option<Vec<Rgba>>)>,
        default: Option<Rgba>,
    },
    InsertLayer {
        index: usize,
        layer: Box<Layer>,
    },
    RemoveLayer {
        index: usize,
    },
    MoveLayer {
        from: usize,
        to: usize,
    },
}

impl Action {
    /// Applies the action to `layers` and returns the action that reverts it.
    fn apply(self, layers: &mut Vec<Layer>) -> Option<Action> {
        match self {
            Action::Tiles {
                layer_id,
                tiles,
                default,
            } => {
                let layer = layers.iter_mut().find(|l| l.id == layer_id)?;
                let tiles = tiles
                    .into_iter()
                    .map(|(key, tile)| (key, layer.swap_tile(key, tile)))
                    .collect();
                let default = default.map(|d| layer.set_default(d));
                Some(Action::Tiles {
                    layer_id,
                    tiles,
                    default,
                })
            }
            Action::InsertLayer { index, mut layer } => {
                layer.mark_all_dirty();
                layers.insert(index.min(layers.len()), *layer);
                Some(Action::RemoveLayer { index })
            }
            Action::RemoveLayer { index } => {
                if index >= layers.len() {
                    return None;
                }
                let layer = Box::new(layers.remove(index));
                Some(Action::InsertLayer { index, layer })
            }
            Action::MoveLayer { from, to } => {
                if from >= layers.len() || to >= layers.len() {
                    return None;
                }
                let layer = layers.remove(from);
                layers.insert(to, layer);
                Some(Action::MoveLayer { from: to, to: from })
            }
        }
    }
}

pub struct History {
    undo: Vec<Action>,
    redo: Vec<Action>,
    limit: usize,
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit,
        }
    }

    /// Records the action that reverts an edit that was just made.
    pub fn push(&mut self, revert: Action) {
        self.redo.clear();
        self.undo.push(revert);
        if self.undo.len() > self.limit {
            self.undo.remove(0);
        }
    }

    /// Performs `action` and records its inverse.
    pub fn perform(&mut self, action: Action, layers: &mut Vec<Layer>) {
        if let Some(revert) = action.apply(layers) {
            self.push(revert);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, layers: &mut Vec<Layer>) {
        if let Some(inv) = self.undo.pop().and_then(|a| a.apply(layers)) {
            self.redo.push(inv);
        }
    }

    pub fn redo(&mut self, layers: &mut Vec<Layer>) {
        if let Some(inv) = self.redo.pop().and_then(|a| a.apply(layers)) {
            self.undo.push(inv);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{PaintMode, PaintOp};

    fn paint(layer: &mut Layer) -> Action {
        let mut op = PaintOp::new(
            layer.id,
            PaintMode::Paint {
                color: [9, 9, 9, 255],
                opacity: 1.0,
            },
        );
        op.stroke_segment(layer, [0.0, 0.0], [0.0, 0.0], 3.0);
        Action::Tiles {
            layer_id: layer.id,
            tiles: op.finish().unwrap(),
            default: None,
        }
    }

    #[test]
    fn undo_redo_pixels() {
        let mut layers = vec![Layer::new(1, "a")];
        let mut h = History::new(10);
        let a = paint(&mut layers[0]);
        h.push(a);
        assert_eq!(layers[0].pixel(0, 0), [9, 9, 9, 255]);
        h.undo(&mut layers);
        assert_eq!(layers[0].pixel(0, 0), [0; 4]);
        assert!(layers[0].tiles.is_empty());
        h.redo(&mut layers);
        assert_eq!(layers[0].pixel(0, 0), [9, 9, 9, 255]);
        assert!(!h.can_redo());
    }

    #[test]
    fn undo_layer_ops() {
        let mut layers = vec![Layer::new(1, "a")];
        let mut h = History::new(10);
        h.perform(
            Action::InsertLayer {
                index: 1,
                layer: Box::new(Layer::new(2, "b")),
            },
            &mut layers,
        );
        h.perform(Action::MoveLayer { from: 1, to: 0 }, &mut layers);
        assert_eq!(layers.iter().map(|l| l.id).collect::<Vec<_>>(), [2, 1]);
        h.perform(Action::RemoveLayer { index: 0 }, &mut layers);
        assert_eq!(layers.len(), 1);
        h.undo(&mut layers);
        h.undo(&mut layers);
        assert_eq!(layers.iter().map(|l| l.id).collect::<Vec<_>>(), [1, 2]);
        h.undo(&mut layers);
        assert_eq!(layers.len(), 1);
        h.redo(&mut layers);
        h.redo(&mut layers);
        assert_eq!(layers.iter().map(|l| l.id).collect::<Vec<_>>(), [2, 1]);
    }

    #[test]
    fn undo_background_fill() {
        let mut layers = vec![Layer::new(1, "a")];
        let mut h = History::new(10);
        let r = crate::canvas::flood_fill(
            &mut layers[0],
            0,
            0,
            0,
            PaintMode::Paint {
                color: [1, 2, 3, 255],
                opacity: 1.0,
            },
        )
        .unwrap();
        h.push(Action::Tiles {
            layer_id: 1,
            tiles: r.tiles,
            default: r.old_default,
        });
        assert_eq!(layers[0].pixel(500, 500), [1, 2, 3, 255]);
        h.undo(&mut layers);
        assert_eq!(layers[0].pixel(500, 500), [0; 4]);
        h.redo(&mut layers);
        assert_eq!(layers[0].pixel(500, 500), [1, 2, 3, 255]);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut layers = vec![Layer::new(1, "a")];
        let mut h = History::new(10);
        let a = paint(&mut layers[0]);
        h.push(a);
        h.undo(&mut layers);
        assert!(h.can_redo());
        let a = paint(&mut layers[0]);
        h.push(a);
        assert!(!h.can_redo());
    }
}
