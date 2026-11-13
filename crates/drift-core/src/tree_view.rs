use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::report::{counts_as_drift, ScanEntry};
use crate::status::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind {
    Dir {
        expanded: bool,
        files: usize,
        drifted: usize,
    },
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRow {
    pub node: usize,
    pub depth: u16,
    pub kind: RowKind,
    pub name: String,
    pub entry: Option<usize>,
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    parent: Option<usize>,
    children: Vec<usize>,
    entry: Option<usize>,
    depth: u16,
}

#[derive(Debug, Clone)]
pub struct TreeView {
    entries: Vec<ScanEntry>,
    nodes: Vec<Node>,
    collapsed: HashSet<usize>,
    status_filter: Option<Status>,
    query: String,
    query_lower: String,
    lowered: Vec<String>,
    descendants: Vec<Vec<usize>>,
    node_files: Vec<usize>,
    node_marked: Vec<usize>,
    node_statuses: Vec<[usize; 6]>,
    counts: [usize; 6],
    drifted: usize,
    marked: BTreeSet<usize>,
    rows: Vec<TreeRow>,
    selected: usize,
}

impl TreeView {
    pub fn new(entries: Vec<ScanEntry>) -> TreeView {
        let (nodes, lowered) = build_nodes(&entries);
        let mut view = TreeView {
            entries,
            nodes,
            collapsed: HashSet::new(),
            status_filter: None,
            query: String::new(),
            query_lower: String::new(),
            lowered,
            descendants: Vec::new(),
            node_files: Vec::new(),
            node_marked: Vec::new(),
            node_statuses: Vec::new(),
            counts: [0; 6],
            drifted: 0,
            marked: BTreeSet::new(),
            rows: Vec::new(),
            selected: 0,
        };
        view.descendants = descendants_of(&view.nodes);
        for entry in &view.entries {
            view.counts[status_index(entry.status)] += 1;
            view.drifted += usize::from(counts_as_drift(entry.status));
        }
        view.collapsed = initial_collapsed(&view.nodes, &view.entries);
        view.rebuild(None);
        view
    }

    pub fn entries(&self) -> &[ScanEntry] {
        &self.entries
    }

    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_row(&self) -> Option<&TreeRow> {
        self.rows.get(self.selected)
    }

    pub fn selected_entry(&self) -> Option<&ScanEntry> {
        self.selected_row()
            .and_then(|row| row.entry)
            .map(|index| &self.entries[index])
    }

    pub fn status_filter(&self) -> Option<Status> {
        self.status_filter
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn counts(&self) -> [usize; 6] {
        self.counts
    }

    pub fn drifted(&self) -> usize {
        self.drifted
    }

    pub fn visible_files(&self) -> usize {
        self.node_files.first().copied().unwrap_or(0)
    }

    pub fn select(&mut self, row: usize) {
        self.selected = row.min(self.rows.len().saturating_sub(1));
    }

    pub fn move_by(&mut self, delta: isize) {
        let target = self.selected as isize + delta;
        self.select(target.max(0) as usize);
    }

    pub fn top(&mut self) {
        self.select(0);
    }

    pub fn bottom(&mut self) {
        self.select(usize::MAX);
    }

    pub fn set_status_filter(&mut self, filter: Option<Status>) {
        let keep = self.selected_node();
        self.status_filter = filter;
        self.rebuild(keep);
    }

    pub fn set_query(&mut self, query: &str) {
        let keep = self.selected_node();
        self.query = query.to_string();
        self.query_lower = query.to_lowercase();
        self.rebuild(keep);
        if self.query_lower.is_empty() {
            return;
        }
        if let Some(at) = self
            .rows
            .iter()
            .position(|row| matches!(row.kind, RowKind::File))
        {
            self.selected = at;
        }
    }

    pub fn toggle(&mut self, row: usize) {
        let Some(found) = self.rows.get(row) else {
            return;
        };
        let RowKind::Dir { expanded, .. } = found.kind else {
            return;
        };
        let node = found.node;
        if expanded {
            self.collapsed.insert(node);
        } else {
            self.collapsed.remove(&node);
        }
        self.rebuild(Some(node));
    }

    pub fn collapse_or_parent(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        match row.kind {
            RowKind::Dir { expanded: true, .. } => self.toggle(self.selected),
            _ => self.select_parent(row.node),
        }
    }

    pub fn expand_or_child(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if let RowKind::Dir { expanded, .. } = row.kind {
            if expanded {
                if self
                    .rows
                    .get(self.selected + 1)
                    .is_some_and(|next| next.depth > row.depth)
                {
                    self.select(self.selected + 1);
                }
            } else {
                self.toggle(self.selected);
            }
        }
    }

    fn select_parent(&mut self, node: usize) {
        let Some(parent) = self.nodes[node].parent else {
            return;
        };
        if let Some(at) = self.rows.iter().position(|row| row.node == parent) {
            self.selected = at;
        }
    }

    pub fn toggle_mark(&mut self, row: usize) {
        let Some(found) = self.rows.get(row) else {
            return;
        };
        let targets = self.files_under(found.node);
        let all = !targets.is_empty() && targets.iter().all(|index| self.marked.contains(index));
        for index in targets {
            if all {
                self.marked.remove(&index);
            } else {
                self.marked.insert(index);
            }
        }
        self.refresh_marks();
    }

    pub fn mark_all_in_view(&mut self) {
        let visible = self.files_under(0);
        let all = !visible.is_empty() && visible.iter().all(|index| self.marked.contains(index));
        for index in visible {
            if all {
                self.marked.remove(&index);
            } else {
                self.marked.insert(index);
            }
        }
        self.refresh_marks();
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
        self.refresh_marks();
    }

    pub fn is_marked(&self, entry: usize) -> bool {
        self.marked.contains(&entry)
    }

    pub fn marked(&self) -> Vec<&ScanEntry> {
        self.marked
            .iter()
            .map(|index| &self.entries[*index])
            .collect()
    }

    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    pub fn paths_under(&self, node: usize) -> Vec<String> {
        let mut paths: Vec<String> = self
            .files_under(node)
            .into_iter()
            .map(|index| self.entries[index].path.clone())
            .collect();
        paths.sort();
        paths
    }

    pub fn dir_status_counts(&self, node: usize) -> [usize; 6] {
        self.node_statuses.get(node).copied().unwrap_or([0; 6])
    }

    pub fn dir_marked(&self, node: usize) -> (usize, usize) {
        (
            self.node_marked.get(node).copied().unwrap_or(0),
            self.node_files.get(node).copied().unwrap_or(0),
        )
    }

    fn files_under(&self, node: usize) -> Vec<usize> {
        match self.nodes[node].entry {
            Some(index) => {
                if self.matches(index) {
                    vec![index]
                } else {
                    Vec::new()
                }
            }
            None => self.descendants[node]
                .iter()
                .copied()
                .filter(|index| self.matches(*index))
                .collect(),
        }
    }

    fn selected_node(&self) -> Option<usize> {
        self.selected_row().map(|row| row.node)
    }

    fn matches(&self, index: usize) -> bool {
        if let Some(filter) = self.status_filter {
            if self.entries[index].status != filter {
                return false;
            }
        }
        self.query_lower.is_empty() || self.lowered[index].contains(&self.query_lower)
    }

    fn refresh_marks(&mut self) {
        let mut marked = vec![0usize; self.nodes.len()];
        for node in (0..self.nodes.len()).rev() {
            let item = &self.nodes[node];
            marked[node] = match item.entry {
                Some(index) => {
                    usize::from(self.node_files[node] == 1 && self.marked.contains(&index))
                }
                None => item.children.iter().map(|child| marked[*child]).sum(),
            };
        }
        self.node_marked = marked;
    }

    fn rebuild(&mut self, keep: Option<usize>) {
        let opening = self.query_lower.is_empty() && self.status_filter.is_none();
        if opening {
            if let Some(node) = keep {
                let show = self
                    .nodes
                    .get(node)
                    .and_then(|item| item.entry)
                    .is_none_or(|index| self.matches(index));
                if show {
                    self.reveal_if_buried(node);
                }
            }
        }
        let query = self.query_lower.clone();
        let mut visible = vec![false; self.nodes.len()];
        let mut files = vec![0usize; self.nodes.len()];
        let mut drifted = vec![0usize; self.nodes.len()];
        let mut statuses = vec![[0usize; 6]; self.nodes.len()];
        for node in (0..self.nodes.len()).rev() {
            let item = &self.nodes[node];
            if let Some(index) = item.entry {
                let entry = &self.entries[index];
                let status_ok = self
                    .status_filter
                    .is_none_or(|filter| entry.status == filter);
                let query_ok = query.is_empty() || self.lowered[index].contains(&query);
                if status_ok && query_ok {
                    visible[node] = true;
                    files[node] = 1;
                    drifted[node] = usize::from(counts_as_drift(entry.status));
                    statuses[node][status_index(entry.status)] = 1;
                }
            } else {
                for child in &item.children {
                    visible[node] |= visible[*child];
                    files[node] += files[*child];
                    drifted[node] += drifted[*child];
                    let child_counts = statuses[*child];
                    for (total, add) in statuses[node].iter_mut().zip(child_counts) {
                        *total += add;
                    }
                }
            }
        }
        let force_open = !query.is_empty() || self.status_filter.is_some();
        let mut rows = Vec::new();
        let mut stack: Vec<usize> = self.nodes[0].children.iter().rev().copied().collect();
        while let Some(node) = stack.pop() {
            if !visible[node] {
                continue;
            }
            let item = &self.nodes[node];
            if item.entry.is_some() {
                rows.push(TreeRow {
                    node,
                    depth: item.depth,
                    kind: RowKind::File,
                    name: item.name.clone(),
                    entry: item.entry,
                });
                continue;
            }
            let expanded = force_open || !self.collapsed.contains(&node);
            rows.push(TreeRow {
                node,
                depth: item.depth,
                kind: RowKind::Dir {
                    expanded,
                    files: files[node],
                    drifted: drifted[node],
                },
                name: item.name.clone(),
                entry: None,
            });
            if expanded {
                stack.extend(item.children.iter().rev().copied());
            }
        }
        self.rows = rows;
        self.node_files = files;
        self.node_statuses = statuses;
        self.refresh_marks();
        let at = keep.and_then(|node| self.rows.iter().position(|row| row.node == node));
        self.selected = at.unwrap_or_else(|| self.selected.min(self.rows.len().saturating_sub(1)));
    }

    fn reveal_if_buried(&mut self, node: usize) {
        let mut buried = false;
        let mut current = self.nodes.get(node).and_then(|item| item.parent);
        while let Some(id) = current {
            if self.collapsed.contains(&id) {
                buried = true;
                break;
            }
            current = self.nodes[id].parent;
        }
        if !buried {
            return;
        }
        current = self.nodes.get(node).and_then(|item| item.parent);
        while let Some(id) = current {
            self.collapsed.remove(&id);
            current = self.nodes[id].parent;
        }
    }
}

fn descendants_of(nodes: &[Node]) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for node in (0..nodes.len()).rev() {
        if nodes[node].entry.is_some() {
            continue;
        }
        let mut all = Vec::new();
        for child in &nodes[node].children {
            match nodes[*child].entry {
                Some(index) => all.push(index),
                None => all.extend(out[*child].iter().copied()),
            }
        }
        out[node] = all;
    }
    out
}

fn opens_on_start(status: Status) -> bool {
    counts_as_drift(status) || status == Status::LocalOverride
}

fn initial_collapsed(nodes: &[Node], entries: &[ScanEntry]) -> HashSet<usize> {
    let mut collapsed = HashSet::new();
    for (id, node) in nodes.iter().enumerate() {
        if id != 0 && node.entry.is_none() {
            collapsed.insert(id);
        }
    }
    let mut stack: Vec<usize> = nodes
        .first()
        .map(|root| root.children.iter().rev().copied().collect())
        .unwrap_or_default();
    let mut target = None;
    while let Some(node) = stack.pop() {
        let item = &nodes[node];
        if let Some(index) = item.entry {
            if opens_on_start(entries[index].status) {
                target = Some(node);
                break;
            }
        } else {
            stack.extend(item.children.iter().rev().copied());
        }
    }
    let mut node = target.unwrap_or(0);
    while let Some(parent) = nodes[node].parent {
        collapsed.remove(&parent);
        node = parent;
    }
    collapsed
}

fn status_index(status: Status) -> usize {
    Status::ALL
        .iter()
        .position(|candidate| *candidate == status)
        .unwrap_or(0)
}

fn build_nodes(entries: &[ScanEntry]) -> (Vec<Node>, Vec<String>) {
    let mut nodes = vec![Node {
        name: String::new(),
        parent: None,
        children: Vec::new(),
        entry: None,
        depth: 0,
    }];
    let mut lookup: BTreeMap<(usize, String), usize> = BTreeMap::new();
    let mut lowered = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        lowered.push(entry.path.to_lowercase());
        let segments: Vec<&str> = entry
            .path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect();
        let mut parent = 0;
        for (depth, segment) in segments.iter().enumerate() {
            let last = depth + 1 == segments.len();
            let key = (parent, segment.to_string());
            let existing = if last {
                None
            } else {
                lookup.get(&key).copied()
            };
            let node = match existing {
                Some(node) => node,
                None => {
                    let id = nodes.len();
                    nodes.push(Node {
                        name: segment.to_string(),
                        parent: Some(parent),
                        children: Vec::new(),
                        entry: last.then_some(index),
                        depth: depth as u16,
                    });
                    nodes[parent].children.push(id);
                    if !last {
                        lookup.insert(key, id);
                    }
                    id
                }
            };
            parent = node;
        }
    }
    for node in 0..nodes.len() {
        let mut children = std::mem::take(&mut nodes[node].children);
        children.sort_by(|left, right| {
            let (a, b) = (&nodes[*left], &nodes[*right]);
            (a.entry.is_some(), &a.name).cmp(&(b.entry.is_some(), &b.name))
        });
        nodes[node].children = children;
    }
    (nodes, lowered)
}

#[cfg(test)]
mod tests {
    use drift_fixtures::cpu_cost;

    use super::*;
    use crate::status::{Badge, BaseProvenance};

    fn entry(path: &str, status: Status) -> ScanEntry {
        ScanEntry {
            path: path.to_string(),
            template: None,
            status,
            badge: None::<Badge>,
            base: BaseProvenance::None,
            volatile: false,
            note: String::new(),
        }
    }

    fn sample() -> TreeView {
        TreeView::new(vec![
            entry(".config/hypr/monitors.lua", Status::Conflict),
            entry(".config/hypr/looknfeel.lua", Status::UpstreamChanged),
            entry(".config/hypr/input.lua", Status::Unchanged),
            entry(".config/waybar/style.css", Status::LocalOverride),
            entry(".config/waybar/config.jsonc", Status::Unchanged),
            entry(".config/btop/btop.conf", Status::Missing),
            entry(".config/alacritty.toml", Status::UserOnly),
        ])
    }

    fn names(view: &TreeView) -> Vec<String> {
        view.rows()
            .iter()
            .map(|row| format!("{}{}", " ".repeat(usize::from(row.depth)), row.name))
            .collect()
    }

    fn expand_all(view: &mut TreeView) {
        while let Some(row) = view.rows().iter().position(|row| {
            matches!(
                row.kind,
                RowKind::Dir {
                    expanded: false,
                    ..
                }
            )
        }) {
            view.toggle(row);
        }
    }

    #[test]
    fn paths_become_a_sorted_tree_with_directories_first() {
        let mut view = sample();
        expand_all(&mut view);
        assert_eq!(
            names(&view),
            [
                ".config",
                " btop",
                "  btop.conf",
                " hypr",
                "  input.lua",
                "  looknfeel.lua",
                "  monitors.lua",
                " waybar",
                "  config.jsonc",
                "  style.css",
                " alacritty.toml",
            ]
        );
        let RowKind::Dir {
            expanded,
            files,
            drifted,
        } = view.rows()[0].kind
        else {
            panic!("a directory");
        };
        assert!(expanded);
        assert_eq!((files, drifted), (7, 3));
        let RowKind::Dir { files, drifted, .. } = view.rows()[3].kind else {
            panic!("a directory");
        };
        assert_eq!((files, drifted), (3, 2));
        assert_eq!(view.rows()[2].entry, Some(5));
    }

    #[test]
    fn counts_follow_the_status_order_and_drift_is_the_report_definition() {
        let view = sample();
        assert_eq!(view.counts(), [2, 1, 1, 1, 1, 1]);
        assert_eq!(view.drifted(), 3);
    }

    #[test]
    fn collapse_and_expand_hide_and_show_children() {
        let mut view = sample();
        expand_all(&mut view);
        view.select(3);
        view.collapse_or_parent();
        assert_eq!(view.rows().len(), 8);
        assert!(matches!(
            view.rows()[3].kind,
            RowKind::Dir {
                expanded: false,
                ..
            }
        ));
        view.expand_or_child();
        assert_eq!(view.rows().len(), 11);
        assert_eq!(view.selected(), 3);
        view.expand_or_child();
        assert_eq!(view.selected(), 4);
        view.collapse_or_parent();
        assert_eq!(view.selected(), 3);
        view.collapse_or_parent();
        assert!(matches!(
            view.rows()[3].kind,
            RowKind::Dir {
                expanded: false,
                ..
            }
        ));
        view.collapse_or_parent();
        assert_eq!(view.selected(), 0);
        view.collapse_or_parent();
        assert_eq!(view.rows().len(), 1);
        view.collapse_or_parent();
        assert_eq!(view.selected(), 0);
    }

    #[test]
    fn toggling_by_row_works_for_directories_only() {
        let mut view = sample();
        expand_all(&mut view);
        view.toggle(2);
        assert_eq!(view.rows().len(), 11);
        view.toggle(1);
        assert_eq!(view.rows().len(), 10);
        view.toggle(99);
        assert_eq!(view.rows().len(), 10);
    }

    #[test]
    fn a_status_filter_keeps_only_matching_files_and_their_directories() {
        let mut view = sample();
        view.set_status_filter(Some(Status::Unchanged));
        assert_eq!(
            names(&view),
            [
                ".config",
                " hypr",
                "  input.lua",
                " waybar",
                "  config.jsonc"
            ]
        );
        assert_eq!(view.status_filter(), Some(Status::Unchanged));
        assert_eq!(view.counts(), [2, 1, 1, 1, 1, 1]);
        view.set_status_filter(Some(Status::UserOnly));
        assert_eq!(names(&view), [".config", " alacritty.toml"]);
        view.set_status_filter(Some(Status::Missing));
        assert_eq!(names(&view), [".config", " btop", "  btop.conf"]);
        view.set_status_filter(None);
        assert_eq!(
            names(&view),
            [
                ".config",
                " btop",
                "  btop.conf",
                " hypr",
                " waybar",
                " alacritty.toml",
            ]
        );
    }

    #[test]
    fn a_filter_with_no_match_leaves_no_rows_and_nothing_selected() {
        let mut view = TreeView::new(vec![entry("a/b.conf", Status::Unchanged)]);
        view.set_status_filter(Some(Status::Conflict));
        assert!(view.rows().is_empty());
        assert!(view.selected_row().is_none());
        assert!(view.selected_entry().is_none());
        view.move_by(3);
        view.bottom();
        view.top();
        assert_eq!(view.selected(), 0);
    }

    #[test]
    fn a_filter_opens_collapsed_directories_and_clearing_restores_them() {
        let mut view = sample();
        view.select(3);
        view.collapse_or_parent();
        view.set_status_filter(Some(Status::Conflict));
        assert_eq!(names(&view), [".config", " hypr", "  monitors.lua"]);
        view.set_status_filter(None);
        assert!(matches!(
            view.rows()[3].kind,
            RowKind::Dir {
                expanded: false,
                ..
            }
        ));
    }

    #[test]
    fn search_is_incremental_case_insensitive_and_matches_the_whole_path() {
        let mut view = sample();
        view.set_query("W");
        assert_eq!(
            names(&view),
            [".config", " waybar", "  config.jsonc", "  style.css"]
        );
        view.set_query("wa");
        assert_eq!(view.visible_files(), 2);
        view.set_query("STYLE");
        assert_eq!(names(&view), [".config", " waybar", "  style.css"]);
        view.set_query("hypr/mon");
        assert_eq!(names(&view), [".config", " hypr", "  monitors.lua"]);
        view.set_query("zzz");
        assert!(view.rows().is_empty());
        view.set_query("");
        assert_eq!(view.query(), "");
        assert_eq!(
            names(&view),
            [
                ".config",
                " btop",
                "  btop.conf",
                " hypr",
                " waybar",
                " alacritty.toml",
            ]
        );
    }

    #[test]
    fn search_and_filter_combine() {
        let mut view = sample();
        view.set_status_filter(Some(Status::Unchanged));
        view.set_query("hypr");
        assert_eq!(names(&view), [".config", " hypr", "  input.lua"]);
        view.set_query("style");
        assert!(view.rows().is_empty());
    }

    #[test]
    fn the_selected_row_follows_its_node_across_filters() {
        let mut view = sample();
        expand_all(&mut view);
        view.select(9);
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/style.css"
        );
        view.set_status_filter(Some(Status::LocalOverride));
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/style.css"
        );
        view.set_status_filter(None);
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/style.css"
        );
    }

    #[test]
    fn a_search_selects_the_first_matching_file() {
        let mut view = sample();
        view.set_query("monitors.lua");
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/hypr/monitors.lua"
        );
        view.set_query("waybar");
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/config.jsonc"
        );
        let style = view
            .rows()
            .iter()
            .position(|row| row.name == "style.css")
            .unwrap();
        view.select(style);
        view.set_query("wayb");
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/config.jsonc"
        );
        view.set_query("");
        assert_eq!(
            view.selected_entry().unwrap().path,
            ".config/waybar/config.jsonc"
        );
        view.set_query("zzz");
        assert!(view.selected_row().is_none());
    }

    #[test]
    fn movement_clamps_and_jumps() {
        let mut view = sample();
        expand_all(&mut view);
        view.select(0);
        view.move_by(-5);
        assert_eq!(view.selected(), 0);
        view.move_by(3);
        assert_eq!(view.selected(), 3);
        view.bottom();
        assert_eq!(view.selected(), 10);
        view.move_by(4);
        assert_eq!(view.selected(), 10);
        view.top();
        assert_eq!(view.selected(), 0);
        view.select(500);
        assert_eq!(view.selected(), 10);
    }

    #[test]
    fn marks_toggle_per_file_and_per_directory_within_the_view() {
        let mut view = sample();
        expand_all(&mut view);
        view.toggle_mark(4);
        assert!(view.is_marked(2));
        assert_eq!(view.marked_count(), 1);
        view.toggle_mark(3);
        assert_eq!(view.marked_count(), 3);
        assert_eq!(view.dir_marked(view.rows()[3].node), (3, 3));
        view.toggle_mark(3);
        assert_eq!(view.marked_count(), 0);
        view.set_status_filter(Some(Status::Unchanged));
        view.toggle_mark(0);
        let paths: Vec<&str> = view
            .marked()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [".config/hypr/input.lua", ".config/waybar/config.jsonc"]
        );
    }

    #[test]
    fn mark_all_applies_to_the_filtered_view_and_toggles_off() {
        let mut view = sample();
        view.set_status_filter(Some(Status::Unchanged));
        view.mark_all_in_view();
        assert_eq!(view.marked_count(), 2);
        view.set_status_filter(None);
        assert_eq!(view.marked_count(), 2);
        view.mark_all_in_view();
        assert_eq!(view.marked_count(), 7);
        view.mark_all_in_view();
        assert_eq!(view.marked_count(), 0);
        view.mark_all_in_view();
        view.clear_marks();
        assert_eq!(view.marked_count(), 0);
        view.set_query("zzz");
        view.mark_all_in_view();
        assert_eq!(view.marked_count(), 0);
    }

    #[test]
    fn five_thousand_files_rebuild_quickly() {
        let entries: Vec<ScanEntry> = (0..5000)
            .map(|n| {
                let status = Status::ALL[n % 6];
                entry(
                    &format!(".config/dir{:03}/file{n:04}.conf", n % 100),
                    status,
                )
            })
            .collect();
        let mut view = TreeView::new(entries);
        assert!(view.rows().len() < 500, "{}", view.rows().len());
        let took = cpu_cost(|| {
            view.set_query("file4");
            view.set_status_filter(Some(Status::Conflict));
            view.set_query("");
            view.set_status_filter(None);
            view.mark_all_in_view();
        });
        assert!(took < std::time::Duration::from_millis(500), "{took:?}");
        assert_eq!(view.marked_count(), 5000);
    }

    #[test]
    fn the_tree_opens_only_as_far_as_the_first_drifted_or_overridden_file() {
        let view = TreeView::new(vec![
            entry(".config/chromium/Default/Bookmarks", Status::UserOnly),
            entry(".config/chromium/Default/Preferences", Status::UserOnly),
            entry(".config/chromium/Default/Cache/data", Status::UserOnly),
            entry(".config/hypr/monitors.lua", Status::Conflict),
            entry(".config/waybar/style.css", Status::LocalOverride),
            entry(".config/alacritty.toml", Status::Unchanged),
        ]);
        assert_eq!(
            names(&view),
            [
                ".config",
                " chromium",
                " hypr",
                "  monitors.lua",
                " waybar",
                " alacritty.toml",
            ]
        );
        assert!(matches!(
            view.rows()[1].kind,
            RowKind::Dir {
                expanded: false,
                ..
            }
        ));
        assert!(matches!(
            view.rows()[2].kind,
            RowKind::Dir { expanded: true, .. }
        ));
        let quiet = TreeView::new(vec![
            entry(".config/chromium/Default/Preferences", Status::UserOnly),
            entry(".config/hypr/input.lua", Status::Unchanged),
        ]);
        assert_eq!(names(&quiet), [".config"]);
        assert!(matches!(
            quiet.rows()[0].kind,
            RowKind::Dir {
                expanded: false,
                ..
            }
        ));
        let earlier = TreeView::new(vec![
            entry(".config/btop/btop.conf", Status::LocalOverride),
            entry(".config/chromium/Default/Preferences", Status::UserOnly),
            entry(".config/hypr/monitors.lua", Status::Conflict),
        ]);
        assert_eq!(
            names(&earlier),
            [".config", " btop", "  btop.conf", " chromium", " hypr"]
        );
    }
}
