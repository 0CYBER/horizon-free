//! The SDK's build-pinned data (`build-data.tar.xz`): `points.json`, `internals.json`,
//! `loops.json` and `functions.json`, written by the staging pass and read by the launcher and the
//! mod tools. Each file opens with the same header, `schema` and `build.commit`, so a file names
//! the game build it describes and a reader refuses a schema it does not know, by name, before
//! reading the rest.
//! No compatibility promise beyond that: the data is exact to its build.
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// The build-data schema this source writes and reads. A new field does not bump it; a removed
/// or redefined one does.
pub const SCHEMA: u32 = 1;
/// The injection points and the `Wire` types their records may hold.
pub const POINTS: &str = "points.json";
/// The staged internals' signatures.
pub const INTERNALS: &str = "internals.json";
/// The published loops.
pub const LOOPS: &str = "loops.json";
/// The function index: where each function a mixin may target is, and whether it can take a
/// replacement.
pub const FUNCTIONS: &str = "functions.json";
/// The four files, in the order the archive holds them.
pub const FILES: [&str; 4] = [FUNCTIONS, INTERNALS, LOOPS, POINTS];
/// The release asset holding them.
pub const ARCHIVE: &str = "build-data.tar.xz";

/// The build a file describes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// The game's full commit sha.
    pub commit: String,
}

/// A build-data file: the header, then the file's own fields.
#[derive(Serialize)]
pub struct Document<'a, T: Serialize> {
    pub schema: u32,
    pub build: &'a Build,
    #[serde(flatten)]
    pub body: T,
}

impl<'a, T: Serialize> Document<'a, T> {
    /// The file for `build` holding `body`, at this source's schema.
    pub fn new(build: &'a Build, body: T) -> Self {
        Self {
            schema: SCHEMA,
            build,
            body,
        }
    }
}

/// Read the build-data file `file` from `text`: refuse a missing or unknown schema by name, then
/// read the build and the body. `T` names the file's own fields; unknown fields are ignored.
pub fn read<T: DeserializeOwned>(file: &str, text: &str) -> Result<(Build, T), String> {
    #[derive(Deserialize)]
    struct Header {
        schema: Option<u64>,
    }
    #[derive(Deserialize)]
    struct WithBuild<T> {
        build: Build,
        #[serde(flatten)]
        body: T,
    }
    let header: Header = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    match header.schema {
        Some(schema) if schema == u64::from(SCHEMA) => {}
        Some(schema) => {
            return Err(format!(
                "{file} has build-data schema {schema}; this program reads schema {SCHEMA}. Use the tool and launcher made for that game build"
            ));
        }
        None => {
            return Err(format!(
                "{file} has no build-data schema; it was written by an older game build. Fetch this build's SDK again"
            ));
        }
    }
    let document: WithBuild<T> = serde_json::from_str(text).map_err(|e| format!("{file}: {e}"))?;
    Ok((document.build, document.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Loops {
        loops: Vec<String>,
    }

    #[test]
    fn a_written_file_reads_back_with_its_build() {
        let build = Build {
            commit: "c".repeat(40),
        };
        let text = serde_json::to_string(&Document::new(
            &build,
            Loops {
                loops: vec!["tick".into()],
            },
        ))
        .unwrap();
        assert!(
            text.starts_with(r#"{"schema":1,"build":{"commit":"#),
            "{text}"
        );
        let (read_build, body) = read::<Loops>(LOOPS, &text).unwrap();
        assert_eq!(read_build, build);
        assert_eq!(body.loops, ["tick"]);
    }

    #[test]
    fn an_unknown_or_missing_schema_is_refused_by_name() {
        let commit = "c".repeat(40);
        let later = format!(r#"{{"schema":2,"build":{{"commit":"{commit}"}},"loops":[]}}"#);
        let error = read::<Loops>(LOOPS, &later).unwrap_err();
        assert!(
            error.contains("loops.json has build-data schema 2"),
            "{error}"
        );
        let error = read::<Loops>(LOOPS, r#"{"loops":[]}"#).unwrap_err();
        assert!(
            error.contains("loops.json has no build-data schema"),
            "{error}"
        );
        let error = read::<Loops>(LOOPS, "[]").unwrap_err();
        assert!(error.starts_with("loops.json: "), "{error}");
        let headless = r#"{"schema":1,"loops":[]}"#;
        assert!(
            read::<Loops>(LOOPS, headless)
                .unwrap_err()
                .contains("build")
        );
    }
}
