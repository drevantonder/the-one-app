//! A `list` (`tree::Kind::List`): a column of `count` rows of which its page gives only those near the window, each by
//! its `index`, as UIKit's collection view and Android's RecyclerView hold only those, so a list of 10,000 is as quick
//! as one of 20. The renderer owns where it is: it lays the rows it has out in their order, holds the room of those
//! before and after them as its padding (each row it has drawn as tall as it was, the rest as tall as the drawn ones
//! are on average, its `estimate` until there's one), and as it's scrolled, asks its page for the rows a window
//! either side of what shows (`range`, `{first, last}`, the last not included), as FlashList's draw distance has it,
//! so a fling finds them drawn.

use std::collections::HashMap;

use serde_json::json;
use taffy::prelude::LengthPercentage;

use super::Screen;
use super::paint::Retained;
use crate::tree::{Id, Kind, Node, Tree};

/// How tall a row is taken to be until its list has drawn one and says no `estimate`: a list row's on a computer.
const ESTIMATE: f32 = 38.0;

/// A row's index in its list.
pub(super) fn index(node: &Node) -> usize {
    node.prop("index").and_then(|index| index.as_u64()).map_or(usize::MAX, |index| index as usize)
}

/// A list's rows as it laid them out.
#[derive(Default)]
pub(crate) struct Listed {
    /// How tall each row it has drawn was, by its index.
    heights: HashMap<usize, f32>,
    /// The room it holds before and after the rows it has, as its padding.
    padding: (f32, f32),
    /// The rows it last asked its page for.
    told: Option<(usize, usize)>,
    /// A row's height changed, so its padding is worked out again in the next frame.
    pub(super) stale: bool,
}

impl Listed {
    /// How many rows it has, and how tall one it hasn't drawn is taken to be.
    fn rows(node: &Node, heights: &HashMap<usize, f32>) -> (usize, f32) {
        let count = node.prop("count").and_then(|count| count.as_u64()).unwrap_or(0) as usize;
        let estimate = match heights.len() {
            0 => node.prop("estimate").and_then(|estimate| estimate.as_f64()).map_or(ESTIMATE, |estimate| estimate as f32),
            drawn => heights.values().sum::<f32>() / drawn as f32,
        };
        (count, estimate)
    }

    /// Where each row starts, from its top, and where the last ends: `count + 1` of them.
    fn offsets(&self, count: usize, estimate: f32) -> Vec<f32> {
        let mut offsets = Vec::with_capacity(count + 1);
        let mut at = 0.0;
        offsets.push(at);
        for row in 0..count {
            at += self.heights.get(&row).copied().unwrap_or(estimate);
            offsets.push(at);
        }
        offsets
    }
}

impl Retained {
    /// Holds list `id`'s room before and after the rows it has as its padding, which its style (styled again or not)
    /// is given here, every frame it's built.
    pub(super) fn hold(&mut self, id: Id, node: &Node, tree: &Tree) {
        let Some(kept) = self.kept.get(&id) else { return };
        let listed = self.lists.entry(id).or_default();
        let (count, estimate) = Listed::rows(node, &listed.heights);
        listed.heights.retain(|row, _| *row < count);
        let offsets = listed.offsets(count, estimate);
        let rows: Vec<usize> = kept.children.iter().filter_map(|child| tree.nodes.get(child)).map(index).filter(|row| *row < count).collect();
        let (first, last) = match (rows.iter().min(), rows.iter().max()) {
            (Some(first), Some(last)) => (*first, last + 1),
            _ => listed.told.map_or((0, 0), |(first, _)| (first.min(count), first.min(count))),
        };
        listed.padding = (offsets[first], offsets[count] - offsets[last]);
        listed.stale = false;
        let (top, bottom) = listed.padding;
        let Ok(style) = self.taffy.style(kept.node) else { return };
        if style.padding.top != LengthPercentage::length(top) || style.padding.bottom != LengthPercentage::length(bottom) {
            let mut style = style.clone();
            (style.padding.top, style.padding.bottom) = (LengthPercentage::length(top), LengthPercentage::length(bottom));
            self.taffy.set_style(kept.node, style).expect("taffy takes any style");
        }
    }

    /// How tall each list's rows were laid out, kept by index. → whether one changed, so the list is laid out again.
    pub(super) fn measure_lists(&mut self, tree: &Tree) -> bool {
        let mut changed = false;
        for (id, listed) in &mut self.lists {
            let Some(kept) = self.kept.get(id) else { continue };
            for child in &kept.children {
                let (Some(node), Some(laid)) = (tree.nodes.get(child), self.kept.get(child).and_then(|kept| self.taffy.layout(kept.node).ok())) else { continue };
                let height = laid.size.height;
                let was = listed.heights.insert(index(node), height);
                if was.is_none_or(|was| (was - height).abs() > 0.5) {
                    listed.stale = true;
                    changed = true;
                }
            }
        }
        changed
    }
}

impl Screen {
    /// The lists whose rows' heights changed, and what they're in, which the next frame builds again.
    pub(super) fn stale_lists(&self) -> Vec<Id> {
        self.layout.lists.iter().filter(|(_, listed)| listed.stale).map(|(id, _)| *id).collect()
    }

    /// Asks each list that listens for `range` for the rows a window either side of what shows of it, where that's
    /// changed, as it was drawn in a window `height` tall.
    pub(super) fn ask_lists(&mut self, height: f32) {
        let mut asks = Vec::new();
        for (id, listed) in &mut self.layout.lists {
            let (Some(node), Some(rect)) = (self.tree.nodes.get(id), self.rects.get(id)) else { continue };
            if node.kind != Kind::List || !node.listens("range") {
                continue;
            }
            let (count, estimate) = Listed::rows(node, &listed.heights);
            let offsets = listed.offsets(count, estimate);
            // What shows of it, from its top, a window either side.
            let (from, to) = (-rect.y - height, -rect.y + 2.0 * height);
            let first = offsets.partition_point(|start| *start <= from).saturating_sub(1).min(count);
            let last = offsets.partition_point(|start| *start < to).min(count).max(first);
            if listed.told != Some((first, last)) {
                listed.told = Some((first, last));
                asks.push((*id, first, last));
            }
        }
        for (id, first, last) in asks {
            self.tell(id, "range", json!({"first": first, "last": last}));
        }
    }
}
