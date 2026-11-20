//! A `list` (`tree::Kind::List`): a column of `count` rows of which its page gives only those near the window, each by
//! its `index`, as UIKit's collection view and Android's RecyclerView hold only those, so a list of 10,000 is as quick
//! as one of 20. The renderer owns where it is: it's as tall as all its rows, and lays each it has at its own place, as
//! a collection view's layout does (each row it has drawn as tall as it was, the rest as tall as the drawn ones are on
//! average, its `estimate` until there's one), so the rows it has needn't be a run: a page reuses a row going out of
//! sight for one coming into it, as RecyclerView does. As it's scrolled it asks its page for the rows it needs
//! (`range`, the last not included): from half a window behind what shows to two ahead of it, the way it's going, so
//! a fling's are drawn before they show, or a window and a quarter each way while it's still, as many rows, so the
//! rows a page made while it was still are all a fling needs; and which of them show (`shownFirst`, `shownLast`), which
//! a page fills first.

use std::collections::HashMap;

use serde_json::json;
use taffy::prelude::{Dimension, LengthPercentageAuto, Position};

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
    /// The rows it last asked its page for, and which of them showed: first, last, shown first, shown last.
    told: Option<[usize; 4]>,
    /// Where it was in the window as it last asked, which says the way it's moving.
    was: Option<f32>,
    /// A row's height changed, so its rows are placed again in the next frame.
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
    /// Lays list `id` out as tall as all its rows, and each row it has at its own place in it, absolutely, which their
    /// styles (styled again or not) are given here, every frame it's built.
    pub(super) fn hold(&mut self, id: Id, node: &Node, tree: &Tree) {
        let Some(kept) = self.kept.get(&id) else { return };
        let listed = self.lists.entry(id).or_default();
        let (count, estimate) = Listed::rows(node, &listed.heights);
        listed.heights.retain(|row, _| *row < count);
        let offsets = listed.offsets(count, estimate);
        listed.stale = false;
        let rows: Vec<(taffy::NodeId, f32)> = kept
            .children
            .iter()
            .filter_map(|child| Some((self.kept.get(child)?.node, offsets[index(tree.nodes.get(child)?).min(count)])))
            .collect();
        let (list, tall) = (kept.node, offsets[count]);
        // It's what its rows are placed in, as a box positioned in CSS is what what's absolute in it is placed in.
        self.restyle_as(list, |style| (style.position, style.size.height) = (Position::Relative, Dimension::length(tall)));
        for (row, top) in rows {
            self.restyle_as(row, |style| {
                style.position = Position::Absolute;
                style.inset.top = LengthPercentageAuto::length(top);
                (style.inset.left, style.inset.right) = (LengthPercentageAuto::length(0.0), LengthPercentageAuto::length(0.0));
            });
        }
    }

    /// Changes Taffy node `node`'s style as `change` does, telling Taffy only where that's a change.
    fn restyle_as(&mut self, node: taffy::NodeId, change: impl FnOnce(&mut taffy::Style)) {
        let Ok(style) = self.taffy.style(node) else { return };
        let mut changed = style.clone();
        change(&mut changed);
        if changed != *style {
            self.taffy.set_style(node, changed).expect("taffy takes any style");
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

    /// Asks each list that listens for `range` for the rows it needs, as it was drawn in a window `height` tall, where
    /// that's changed: from half a window behind what shows to two ahead, the way it's moving, or a window and a
    /// quarter each way; near its start or end, as many rows, those it has none of on one side on the other, so the
    /// rows a scroll from there needs are made while it's still, as a collection view's prefetching makes them ahead.
    pub(super) fn ask_lists(&mut self, height: f32) {
        let mut asks = Vec::new();
        for (id, listed) in &mut self.layout.lists {
            let (Some(node), Some(rect)) = (self.tree.nodes.get(id), self.rects.get(id)) else { continue };
            if node.kind != Kind::List || !node.listens("range") {
                continue;
            }
            let (count, estimate) = Listed::rows(node, &listed.heights);
            let offsets = listed.offsets(count, estimate);
            let starting = |at: f32| offsets.partition_point(|start| *start <= at).saturating_sub(1).min(count);
            let ending = |at: f32| offsets.partition_point(|start| *start < at).min(count);
            // What shows of it, from its top, and the way it's going: up the window (on down the list), or down it.
            let (top, bottom) = (-rect.y, -rect.y + height);
            let (behind, ahead) = match listed.was.map(|was| rect.y - was) {
                Some(moved) if moved < -0.5 => (height / 2.0, 2.0 * height),
                Some(moved) if moved > 0.5 => (2.0 * height, height / 2.0),
                _ => (1.25 * height, 1.25 * height),
            };
            let (before, after) = ((behind - top).max(0.0), (bottom + ahead - offsets[count]).max(0.0));
            let (behind, ahead) = (behind + after, ahead + before);
            listed.was = Some(rect.y);
            let (first, shown) = (starting(top - behind), starting(top));
            let told = [first, ending(bottom + ahead).max(first), shown, ending(bottom).max(shown)];
            if listed.told != Some(told) {
                listed.told = Some(told);
                asks.push((*id, told));
            }
        }
        for (id, [first, last, shown_first, shown_last]) in asks {
            self.tell(id, "range", json!({"first": first, "last": last, "shownFirst": shown_first, "shownLast": shown_last}));
        }
    }
}
