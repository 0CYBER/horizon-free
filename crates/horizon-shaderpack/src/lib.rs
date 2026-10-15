//! Java shader packs (OptiFine/Iris) for Horizon (docs/shader-packs.md,
//! "Java shader packs"). This crate does the part that needs no GPU: open a
//! pack, find its programs and options, and turn a program into plain GLSL
//! the way Iris does before compiling it (`#include`s inlined, the player's
//! option values written into the source, Iris's macros expanded).

pub mod include;
pub mod options;
pub mod preprocess;
pub mod source;

use include::{IncludeGraph, Inlined};
use options::{Annotated, OptionSet, OptionValues};
use preprocess::{Environment, Preprocessed};
use source::PackFiles;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

/// A program's pipeline stage, from its file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Fragment,
    Geometry,
    Compute,
    TessControl,
    TessEval,
}

impl Stage {
    /// The stage of `composite1.fsh` and the like; `None` for other files.
    pub fn of(program: &str) -> Option<Self> {
        Some(match program.rsplit_once('.')?.1 {
            "vsh" => Self::Vertex,
            "fsh" => Self::Fragment,
            "gsh" => Self::Geometry,
            "csh" => Self::Compute,
            "tcs" => Self::TessControl,
            "tes" => Self::TessEval,
            _ => return None,
        })
    }
}

/// An opened Java shader pack.
pub struct JavaPack {
    pub files: PackFiles,
    /// Every program source file (`composite1.fsh`, `world-1/final.vsh`).
    pub programs: Vec<String>,
    pub graph: IncludeGraph,
    annotations: BTreeMap<String, Annotated>,
    /// The options the player can set (`ShaderPackOptions`).
    pub options: OptionSet,
}

impl JavaPack {
    /// Open the pack folder or zip at `path` and read every program and file
    /// they include.
    pub fn open(path: &Path) -> Result<Self, String> {
        let files = PackFiles::open(path)?;
        let programs = files.programs();
        if programs.is_empty() {
            return Err("no programs in shaders/".into());
        }
        let graph = IncludeGraph::build(&files, &programs);
        let annotations: BTreeMap<String, Annotated> = graph
            .files
            .iter()
            .map(|(path, node)| (path.clone(), Annotated::parse(&node.lines)))
            .collect();
        // A boolean #define is an option when an #ifdef in its component
        // refers to it.
        let mut options = OptionSet::default();
        for component in graph.components() {
            let refs: BTreeSet<&String> = component
                .iter()
                .flat_map(|f| &annotations[f].ifdef_refs)
                .collect();
            for file in &component {
                let a = &annotations[file];
                a.bools
                    .values()
                    .filter(|o| refs.contains(&o.name))
                    .for_each(|o| options.add_bool(o));
                a.strings.values().for_each(|o| options.add_string(o));
            }
        }
        Ok(Self {
            files,
            programs,
            graph,
            annotations,
            options,
        })
    }

    /// The player's values from saved `name=value` pairs (`shaderpacks/<pack>.txt`).
    pub fn values(&self, saved: &BTreeMap<String, String>) -> OptionValues {
        OptionValues::new(&self.options, saved)
    }

    /// `program` with its includes inlined and `values` applied.
    pub fn source(&self, program: &str, values: &OptionValues) -> Result<Inlined, String> {
        self.graph
            .inline(program, &|file, i, line| match self.annotations.get(file) {
                Some(a) => a.edit(values, i, line),
                None => line.to_owned(),
            })
    }

    /// `program` as plain GLSL: [`JavaPack::source`], then the preprocessor.
    pub fn preprocess(
        &self,
        program: &str,
        values: &OptionValues,
        env: &Environment,
    ) -> Result<Preprocessed, String> {
        preprocess::preprocess(&self.source(program, values)?, env)
    }
}
