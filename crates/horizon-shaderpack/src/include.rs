//! `#include` the way Iris resolves it (`IncludeGraph`, `IncludeProcessor`):
//! before any other directive, so include guards don't apply; `/lib/x.glsl`
//! from `shaders/`, `x.glsl` from the including file's folder. Every file
//! reachable from the programs is read once into a graph, and a program is
//! the graph inlined from its file.

use crate::source::PackFiles;
use std::collections::{BTreeMap, BTreeSet};

/// One file of the graph: its lines and the include on each `#include` line.
pub struct FileNode {
    pub lines: Vec<String>,
    /// Line index → included file (a normalised path relative to `shaders/`).
    pub includes: BTreeMap<usize, String>,
}

pub struct IncludeGraph {
    pub files: BTreeMap<String, FileNode>,
    /// Files that couldn't be read or include themselves, with the reason.
    pub failures: BTreeMap<String, String>,
}

/// A program after inlining: the source and, for each of its lines, the
/// file and 1-based line it came from (to report errors where they are).
pub struct Inlined {
    pub text: String,
    pub origin: Vec<(String, u32)>,
}

impl Inlined {
    /// `file:line` of 1-based `line` of the inlined text.
    pub fn locate(&self, line: u32) -> String {
        match self.origin.get(line.saturating_sub(1) as usize) {
            Some((file, at)) => format!("{file}:{at}"),
            None => format!("line {line}"),
        }
    }
}

/// `target` from an `#include` in `from`, as a path relative to `shaders/`
/// with `.` and `..` resolved (Iris's `AbsolutePackPath`).
pub fn resolve(from: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_owned(),
        None => match from.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{target}"),
            None => target.to_owned(),
        },
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The target of an `#include` line, or `None` for any other line.
fn include_target(line: &str) -> Option<&str> {
    let t = line.trim();
    if !t.starts_with("#include") {
        return None;
    }
    let target = t.get("#include ".len()..).unwrap_or("").trim();
    let target = target.strip_prefix('"').unwrap_or(target);
    Some(target.strip_suffix('"').unwrap_or(target))
}

impl IncludeGraph {
    /// Read `starts` and every file they include, transitively.
    pub fn build(files: &PackFiles, starts: &[String]) -> Self {
        let mut graph = Self {
            files: BTreeMap::new(),
            failures: BTreeMap::new(),
        };
        let mut seen: BTreeSet<String> = starts.iter().cloned().collect();
        let mut queue: Vec<(String, Option<(String, usize)>)> =
            starts.iter().map(|s| (s.clone(), None)).collect();
        while let Some((path, from)) = queue.pop() {
            let Some(text) = files.read(&path) else {
                let why = match from {
                    Some((file, line)) => {
                        format!("{file}:{}: #include \"{path}\": file not found", line + 1)
                    }
                    None => format!("{path}: file not found"),
                };
                graph.failures.insert(path, why);
                continue;
            };
            let lines: Vec<String> = text.lines().map(str::to_owned).collect();
            let includes: BTreeMap<usize, String> = lines
                .iter()
                .enumerate()
                .filter_map(|(i, l)| Some((i, resolve(&path, include_target(l)?))))
                .collect();
            if let Some((line, _)) = includes.iter().find(|(_, target)| **target == path) {
                graph.failures.insert(
                    path.clone(),
                    format!("{path}:{}: file includes itself", line + 1),
                );
                continue;
            }
            for (&line, target) in &includes {
                if seen.insert(target.clone()) {
                    queue.push((target.clone(), Some((path.clone(), line))));
                }
            }
            graph.files.insert(path, FileNode { lines, includes });
        }
        graph
    }

    /// The graph's weakly connected components (files linked by includes in
    /// either direction), each sorted. Iris discovers options per component.
    pub fn components(&self) -> Vec<Vec<String>> {
        let names: Vec<&String> = self.files.keys().collect();
        let index: BTreeMap<&str, usize> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.as_str(), i))
            .collect();
        let mut parent: Vec<usize> = (0..names.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (i, node) in self.files.values().enumerate() {
            for target in node.includes.values() {
                if let Some(&j) = index.get(target.as_str()) {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
        let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            let r = root(&mut parent, i);
            groups.entry(r).or_default().push((*name).clone());
        }
        groups.into_values().collect()
    }

    /// Inline `start`, passing every line that isn't an `#include` through
    /// `edit(file, line index, text)` first (Iris applies option values there).
    pub fn inline(
        &self,
        start: &str,
        edit: &dyn Fn(&str, usize, &str) -> String,
    ) -> Result<Inlined, String> {
        let mut out = Inlined {
            text: String::new(),
            origin: Vec::new(),
        };
        let mut stack = Vec::new();
        self.inline_into(start, edit, &mut stack, &mut out)?;
        Ok(out)
    }

    fn inline_into(
        &self,
        path: &str,
        edit: &dyn Fn(&str, usize, &str) -> String,
        stack: &mut Vec<String>,
        out: &mut Inlined,
    ) -> Result<(), String> {
        if let Some(why) = self.failures.get(path) {
            return Err(why.clone());
        }
        if stack.iter().any(|p| p == path) {
            return Err(format!("#include cycle: {} -> {path}", stack.join(" -> ")));
        }
        let node = self
            .files
            .get(path)
            .ok_or_else(|| format!("{path}: file not found"))?;
        stack.push(path.to_owned());
        for (i, line) in node.lines.iter().enumerate() {
            match node.includes.get(&i) {
                Some(target) => self.inline_into(target, edit, stack, out)?,
                None => {
                    out.text.push_str(&edit(path, i, line));
                    out.text.push('\n');
                    out.origin.push((path.to_owned(), i as u32 + 1));
                }
            }
        }
        stack.pop();
        Ok(())
    }
}
