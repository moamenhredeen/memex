//! Node graph view — force-directed graph of vault notes and [[wikilink]] connections.

mod view;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::*;

use crate::command::Command;
use crate::markdown::{self, StyleKind};
use crate::minibuffer::Candidate;
use crate::pane::ItemAction;

pub use view::GraphView;

// ─── Data ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub title: String,
    pub path: PathBuf,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub source: usize,
    pub target: usize,
}

// ─── State ──────────────────────────────────────────────────────────────────

pub struct GraphState {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
    pub selected: Option<usize>,
    pub hovered: Option<usize>,
    pub local_mode: bool,
    pub local_root: Option<usize>,
    pub focus_handle: FocusHandle,
    /// Map from note stem (lowercase, no extension) → node index for fast lookup.
    title_index: HashMap<String, usize>,
    /// Whether physics simulation is active.
    sim_active: bool,
    sim_steps: usize,
}

impl GraphState {
    pub fn new(cx: &mut App) -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
            selected: None,
            hovered: None,
            local_mode: false,
            local_root: None,
            focus_handle: cx.focus_handle(),
            title_index: HashMap::new(),
            sim_active: true,
            sim_steps: 0,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    // ─── Graph building ─────────────────────────────────────────────────

    /// Build graph from vault notes by scanning files for [[wikilinks]].
    pub fn build_from_vault(&mut self, vault_path: &Path, notes: &[PathBuf]) {
        self.nodes.clear();
        self.edges.clear();
        self.title_index.clear();
        self.selected = None;
        self.hovered = None;
        self.sim_active = true;
        self.sim_steps = 0;

        // Create nodes
        for (i, note_path) in notes.iter().enumerate() {
            let title = note_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
                .to_string();
            let key = title.to_lowercase();

            // Scatter initial positions in a circle
            let angle = (i as f32 / notes.len().max(1) as f32) * std::f32::consts::TAU;
            let radius = 150.0 + (notes.len() as f32).sqrt() * 30.0;

            self.nodes.push(GraphNode {
                title: title.clone(),
                path: note_path.clone(),
                x: angle.cos() * radius,
                y: angle.sin() * radius,
                vx: 0.0,
                vy: 0.0,
            });
            self.title_index.insert(key, i);
        }

        // Scan each note for [[wikilinks]] and create edges
        for (source_idx, note_path) in notes.iter().enumerate() {
            let full_path = if note_path.is_absolute() {
                note_path.clone()
            } else {
                vault_path.join(note_path)
            };

            if let Ok(content) = std::fs::read_to_string(&full_path) {
                let links = extract_wikilinks(&content);
                for link_target in links {
                    let key = link_target.to_lowercase();
                    if let Some(&target_idx) = self.title_index.get(&key) {
                        if source_idx != target_idx {
                            // Avoid duplicate edges
                            let exists = self.edges.iter().any(|e| {
                                (e.source == source_idx && e.target == target_idx)
                                    || (e.source == target_idx && e.target == source_idx)
                            });
                            if !exists {
                                self.edges.push(GraphEdge {
                                    source: source_idx,
                                    target: target_idx,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // ─── Physics simulation ─────────────────────────────────────────────

    /// Run one physics tick. Returns true if nodes moved significantly.
    pub fn tick(&mut self) -> bool {
        if !self.sim_active || self.nodes.is_empty() {
            return false;
        }

        const REPULSION: f32 = 5000.0;
        const ATTRACTION: f32 = 0.01;
        const IDEAL_LEN: f32 = 120.0;
        const DAMPING: f32 = 0.85;
        const MIN_VELOCITY: f32 = 0.1;
        const MAX_STEPS: usize = 300;

        let n = self.nodes.len();
        let mut forces = vec![(0.0f32, 0.0f32); n];

        // Repulsion: all pairs
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = self.nodes[i].x - self.nodes[j].x;
                let dy = self.nodes[i].y - self.nodes[j].y;
                let dist_sq = (dx * dx + dy * dy).max(1.0);
                let dist = dist_sq.sqrt();
                let force = REPULSION / dist_sq;
                let fx = (dx / dist) * force;
                let fy = (dy / dist) * force;
                forces[i].0 += fx;
                forces[i].1 += fy;
                forces[j].0 -= fx;
                forces[j].1 -= fy;
            }
        }

        // Attraction: connected pairs
        for edge in &self.edges {
            let dx = self.nodes[edge.target].x - self.nodes[edge.source].x;
            let dy = self.nodes[edge.target].y - self.nodes[edge.source].y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let displacement = dist - IDEAL_LEN;
            let force = ATTRACTION * displacement;
            let fx = (dx / dist) * force;
            let fy = (dy / dist) * force;
            forces[edge.source].0 += fx;
            forces[edge.source].1 += fy;
            forces[edge.target].0 -= fx;
            forces[edge.target].1 -= fy;
        }

        // Center gravity (weak pull toward origin)
        for i in 0..n {
            forces[i].0 -= self.nodes[i].x * 0.001;
            forces[i].1 -= self.nodes[i].y * 0.001;
        }

        // Apply forces
        let mut max_v: f32 = 0.0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            node.vx = (node.vx + forces[i].0) * DAMPING;
            node.vy = (node.vy + forces[i].1) * DAMPING;
            node.x += node.vx;
            node.y += node.vy;
            max_v = max_v.max(node.vx.abs()).max(node.vy.abs());
        }

        self.sim_steps += 1;
        if max_v < MIN_VELOCITY || self.sim_steps > MAX_STEPS {
            self.sim_active = false;
        }

        max_v > MIN_VELOCITY
    }

    /// Get the set of node indices visible in local mode.
    pub fn local_visible_set(&self) -> Option<Vec<usize>> {
        if !self.local_mode {
            return None;
        }
        let root = self.local_root?;
        let mut visible = vec![root];
        for edge in &self.edges {
            if edge.source == root {
                visible.push(edge.target);
            } else if edge.target == root {
                visible.push(edge.source);
            }
        }
        Some(visible)
    }

    /// Find node at screen coordinates.
    pub fn node_at(&self, sx: f32, sy: f32, cx_w: f32, cy_h: f32) -> Option<usize> {
        let node_radius = 8.0 * self.zoom;
        let hit_radius = node_radius + 4.0;

        let visible = self.local_visible_set();

        for (i, node) in self.nodes.iter().enumerate() {
            if let Some(ref vis) = visible {
                if !vis.contains(&i) {
                    continue;
                }
            }
            let nx = node.x * self.zoom + self.pan_x + cx_w / 2.0;
            let ny = node.y * self.zoom + self.pan_y + cy_h / 2.0;
            let dx = sx - nx;
            let dy = sy - ny;
            if dx * dx + dy * dy < hit_radius * hit_radius {
                return Some(i);
            }
        }
        None
    }

    /// Set local mode centered on a specific note (by path).
    pub fn set_local_root_by_path(&mut self, path: &Path) {
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        self.local_root = self.title_index.get(&stem).copied();
        self.local_mode = self.local_root.is_some();
    }

    // ─── PaneItem interface ─────────────────────────────────────────────

    pub fn commands() -> Vec<Command> {
        vec![
            Command {
                id: "zoom-in",
                name: "Graph: Zoom In",
                description: "Zoom into the graph",
                aliases: &[],
                binding: Some("+"),
            },
            Command {
                id: "zoom-out",
                name: "Graph: Zoom Out",
                description: "Zoom out of the graph",
                aliases: &[],
                binding: Some("-"),
            },
            Command {
                id: "reset-zoom",
                name: "Graph: Reset Zoom",
                description: "Reset zoom to 100%",
                aliases: &[],
                binding: Some("0"),
            },
            Command {
                id: "center-graph",
                name: "Graph: Center",
                description: "Center the graph view",
                aliases: &[],
                binding: Some("c"),
            },
            Command {
                id: "toggle-local-graph",
                name: "Graph: Toggle Local/Global",
                description: "Toggle between local (current note) and global graph",
                aliases: &[],
                binding: Some("l"),
            },
            Command {
                id: "close-graph",
                name: "Graph: Close",
                description: "Close the graph panel",
                aliases: &[],
                binding: Some("q"),
            },
        ]
    }

    pub fn execute_command(
        &mut self,
        cmd_id: &str,
        _viewport: (f32, f32),
        _vim_enabled: bool,
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        match cmd_id {
            "zoom-in" => {
                self.zoom = (self.zoom * 1.2).min(5.0);
                vec![]
            }
            "zoom-out" => {
                self.zoom = (self.zoom / 1.2).max(0.1);
                vec![]
            }
            "reset-zoom" => {
                self.zoom = 1.0;
                vec![]
            }
            "center-graph" => {
                self.pan_x = 0.0;
                self.pan_y = 0.0;
                vec![]
            }
            "toggle-local-graph" => {
                if self.local_mode {
                    self.local_mode = false;
                    vec![ItemAction::SetMessage("Global graph".into())]
                } else if self.local_root.is_some() {
                    self.local_mode = true;
                    vec![ItemAction::SetMessage("Local graph".into())]
                } else {
                    vec![ItemAction::SetMessage("No root note selected".into())]
                }
            }
            "close-graph" => {
                // The app handles this by checking for the close action
                vec![ItemAction::SetMessage("__close_split__".into())]
            }
            _ => vec![],
        }
    }

    pub fn get_candidates(&self, _delegate_id: &str, _input: &str) -> Vec<Candidate> {
        vec![]
    }

    pub fn handle_confirm(
        &mut self,
        _delegate_id: &str,
        _input: &str,
        _candidate: Option<&Candidate>,
        _cx: &mut Context<Self>,
    ) -> Vec<ItemAction> {
        vec![]
    }

    pub fn on_input_changed(&mut self, _delegate_id: &str, _input: &str, _cx: &mut Context<Self>) {}
}

impl EventEmitter<GraphEvent> for GraphState {}

pub enum GraphEvent {
    /// User clicked a node — request the app to open this note.
    OpenNote(PathBuf),
}

// ─── Wikilink extraction ────────────────────────────────────────────────────

/// Extract wikilink targets from markdown content.
fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    for line in content.lines() {
        let spans = markdown::parse_inline_styles(line);
        for span in spans {
            if span.kind == StyleKind::Wikilink {
                // Span range includes [[ and ]], extract inner text
                let raw = &line[span.range.clone()];
                if let Some(inner) = raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
                    // Handle [[target|display]] format
                    let target = inner.split('|').next().unwrap_or(inner).trim();
                    if !target.is_empty() {
                        links.push(target.to_string());
                    }
                }
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::extract_wikilinks;

    #[test]
    fn test_extract_wikilinks() {
        let content = "See [[my note]] and [[other page|display text]] here.";
        let links = extract_wikilinks(content);
        assert_eq!(links, vec!["my note", "other page"]);
    }

    #[test]
    fn test_extract_wikilinks_empty() {
        let content = "No links here.";
        let links = extract_wikilinks(content);
        assert!(links.is_empty());
    }

    #[test]
    fn test_extract_wikilinks_multiline() {
        let content = "# Title\n\nSee [[note a]].\n\nAlso [[note b]].\n";
        let links = extract_wikilinks(content);
        assert_eq!(links, vec!["note a", "note b"]);
    }
}
