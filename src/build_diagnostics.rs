use std::{path::Path, sync::OnceLock};

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    pub location: Option<String>,
    pub line: Option<usize>,
    pub import_line: Option<usize>,
    pub help: String,
    pub resource: Option<String>,
}

#[derive(Debug)]
pub struct BuildDiagnostics {
    pub problems: Vec<Problem>,
    pub source: String,
    pub proposed: bool,
    pub selected: usize,
}

impl BuildDiagnostics {
    pub fn parse(log: &str, source: String, path: &Path, proposed: bool) -> Self {
        let mut problems = Vec::new();
        for message in log.lines().filter_map(|line| line.strip_prefix("error: ")) {
            // Tectonic appends an engine summary after the actual error.
            if message.starts_with("something bad happened")
                || message == "the XeTeX engine had an unrecoverable error"
                || message == "the xdvipdfmx engine had an unrecoverable error"
            {
                continue;
            }
            let mut problem = Problem::parse(message, path, source.lines().count());
            problem.locate_import(&source, path);
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }

        // xdvipdfmx reports a missing physical font as warnings before its fatal summary.
        if let Some(name) = log
            .lines()
            .find_map(|line| {
                line.strip_prefix("warning: >> This font is mapped to a physical font \"")
                    .and_then(|name| name.strip_suffix("\"."))
            })
            .filter(|name| texo::sandbox::is_bundle_resource(name))
        {
            problems.insert(
                0,
                Problem {
                    message: format!("Font file '{name}' is unavailable offline."),
                    location: None,
                    line: None,
                    import_line: None,
                    help: "Download this font resource, then retry the offline build.".into(),
                    resource: Some(name.into()),
                },
            );
        }

        if problems.is_empty() {
            let message = if log.contains("Install Tectonic") || log.contains("Could not start") {
                "A required build tool could not be started."
            } else if log.contains("time limit") {
                "The build timed out."
            } else if log.contains("Local LaTeX resources are missing") {
                "The local LaTeX resource cache is missing."
            } else {
                "The PDF build failed."
            };
            let mut problem = Problem::parse(message, path, source.lines().count());
            problem.help =
                "Open Details for the underlying error; retry after resolving it.".into();
            if log.contains("Install Tectonic") {
                problem.help =
                    "Install Tectonic and ensure it is available on PATH, then retry.".into();
            } else if log.contains("Local LaTeX resources are missing") {
                problem.resource = Some("latex.ltx".into());
                problem.help = "Download the core LaTeX resource, then retry. Additional resources may be needed.".into();
            }
            problems.push(problem);
        }

        Self {
            problems,
            source,
            proposed,
            selected: 0,
        }
    }

    pub fn selected(&self) -> &Problem {
        &self.problems[self.selected]
    }
}

impl Problem {
    pub fn editor_line(&self) -> Option<usize> {
        self.line.or(self.import_line)
    }

    fn locate_import(&mut self, source: &str, path: &Path) {
        if self.line.is_some() {
            return;
        }
        let Some((file, _)) = self
            .location
            .as_deref()
            .and_then(|location| location.rsplit_once(':'))
        else {
            return;
        };
        static IMPORT: OnceLock<Regex> = OnceLock::new();
        // Only literal, standalone imports can be attributed without interpreting TeX macros.
        let pattern =
            IMPORT.get_or_init(|| Regex::new(r"^\s*\\(?:input|include)\s*\{([^{}]+)\}").unwrap());
        let dependency = file.strip_suffix(".tex").unwrap_or(file);
        let mut matches = source.lines().enumerate().filter_map(|(index, line)| {
            let captures = pattern.captures(line)?;
            let imported = captures[1].trim();
            let imported = imported.strip_suffix(".tex").unwrap_or(imported);
            let resolved = path.parent().unwrap_or(Path::new(".")).join(imported);
            (imported == dependency || resolved == Path::new(dependency)).then_some(index + 1)
        });
        let first = matches.next();
        if matches.next().is_none() {
            self.import_line = first;
        }
    }

    fn parse(message: &str, path: &Path, line_count: usize) -> Self {
        static LOCATION: OnceLock<Regex> = OnceLock::new();
        static MISSING: OnceLock<Regex> = OnceLock::new();
        let location_pattern =
            LOCATION.get_or_init(|| Regex::new(r"^(.+?):(\d+):\s*(.*)$").unwrap());
        let missing_pattern =
            MISSING.get_or_init(|| Regex::new(r"File [`']([^`']+)' not found").unwrap());

        let mut problem = Self {
            message: message.trim_start_matches('!').trim_start().into(),
            location: None,
            line: None,
            import_line: None,
            help: "Review the source at this location, or ask AI for a proposed fix.".into(),
            resource: None,
        };
        if let Some(captures) = location_pattern.captures(message) {
            let file = Path::new(&captures[1]);
            let generated_main = file
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".texo-agent-") && name.ends_with(".tex"));
            let is_main = generated_main || file == path;
            let line = captures[2].parse::<usize>().ok().filter(|line| *line > 0);
            problem.message = captures[3].trim_start_matches('!').trim_start().into();
            problem.location = Some(format!(
                "{}:{}",
                if is_main {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                } else {
                    captures[1].to_string()
                },
                &captures[2]
            ));
            if is_main {
                problem.line = line.filter(|line| *line <= line_count);
            }
        }

        if let Some(captures) = missing_pattern.captures(&problem.message) {
            let name = captures[1].to_string();
            if texo::sandbox::is_bundle_resource(&name) {
                problem.resource = Some(name.clone());
                problem.help = format!(
                    "{name} is unavailable offline. Download this resource and retry; more dependencies may be needed."
                );
            } else {
                problem.help = format!(
                    "Check that {name} exists in the project and that its path and capitalization match."
                );
            }
        } else if problem.message.contains("File ended while scanning") {
            problem.help =
                "Check for a missing closing brace or an unfinished command argument.".into();
        } else if problem.message.contains("Undefined control sequence") {
            problem.help = "Check the command spelling and required package. Tectonic uses XeTeX; pdfTeX-only commands may need replacing.".into();
        }

        problem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_current_main_file_and_deduplicates_errors() {
        let log = concat!(
            "error: .texo-agent-123.tex:2: ! Undefined control sequence.\n",
            "error: .texo-agent-123.tex:2: ! Undefined control sequence.\n",
            "error: glyphtounicode:7: Undefined control sequence.\n",
            "error: something bad happened inside XeTeX\n",
            "error: the XeTeX engine had an unrecoverable error",
        );
        let diagnostics =
            BuildDiagnostics::parse(log, "one\ntwo".into(), Path::new("main.tex"), false);
        assert_eq!(diagnostics.problems.len(), 2);
        assert_eq!(diagnostics.problems[0].line, Some(2));
        assert_eq!(
            diagnostics.problems[0].location.as_deref(),
            Some("main.tex:2")
        );
        assert_eq!(diagnostics.problems[1].line, None);
    }

    #[test]
    fn dependency_errors_point_to_unique_literal_imports_without_changing_the_error_location() {
        let diagnostics = BuildDiagnostics::parse(
            "error: glyphtounicode:7: Undefined control sequence",
            "% \\input{glyphtounicode}\n\\input{glyphtounicode}\n".into(),
            Path::new("main.tex"),
            false,
        );
        assert_eq!(
            diagnostics.selected().location.as_deref(),
            Some("glyphtounicode:7")
        );
        assert_eq!(diagnostics.selected().line, None);
        assert_eq!(diagnostics.selected().import_line, Some(2));
    }

    #[test]
    fn ambiguous_or_commented_imports_do_not_guess_a_source_line() {
        for source in [
            "% \\input{support}",
            "\\input{support}\n\\input{support.tex}",
            "\\newcommand{\\load}{\\input{support}}",
        ] {
            let diagnostics = BuildDiagnostics::parse(
                "error: support.tex:7: Undefined control sequence",
                source.into(),
                Path::new("main.tex"),
                false,
            );
            assert_eq!(diagnostics.selected().editor_line(), None);
        }
    }

    #[test]
    fn distinguishes_missing_resources_from_project_files() {
        let package = Problem::parse("File `fontenc.sty' not found", Path::new("main.tex"), 1);
        assert_eq!(package.resource.as_deref(), Some("fontenc.sty"));

        let asset = Problem::parse("File `portrait.png' not found", Path::new("main.tex"), 1);
        assert_eq!(asset.resource, None);
        assert!(asset.help.contains("project"));
    }

    #[test]
    fn missing_physical_font_offers_download_without_a_fake_source_location() {
        let diagnostics = BuildDiagnostics::parse(
            "warning: >> This font is mapped to a physical font \"lmsy9.pfb\".\nerror: the xdvipdfmx engine had an unrecoverable error",
            "source".into(),
            Path::new("main.tex"),
            false,
        );
        assert_eq!(
            diagnostics.selected().resource.as_deref(),
            Some("lmsy9.pfb")
        );
        assert_eq!(diagnostics.selected().line, None);
    }

    #[test]
    fn unfinished_arguments_explain_the_problem_without_inventing_a_line() {
        let diagnostics = BuildDiagnostics::parse(
            "error: !File ended while scanning use of \\textbf",
            "\\textbf{unfinished".into(),
            Path::new("main.tex"),
            false,
        );
        assert_eq!(diagnostics.selected().editor_line(), None);
        assert!(diagnostics.selected().message.starts_with("File ended"));
        assert!(diagnostics.selected().help.contains("closing brace"));
    }

    #[test]
    fn unknown_errors_and_out_of_bounds_locations_do_not_invent_editor_lines() {
        let diagnostics = BuildDiagnostics::parse(
            "sandbox failed",
            "text".into(),
            Path::new("main.tex"),
            false,
        );
        assert_eq!(diagnostics.selected().line, None);
        let invalid = Problem::parse(".texo-agent-123.tex:99: broken", Path::new("main.tex"), 1);
        assert_eq!(invalid.line, None);
    }
}
