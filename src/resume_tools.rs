use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, anyhow, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    llm::AssistantReply,
    tools::{ToolDefinition, ToolSet},
};

pub const MAX_DRAFT_BYTES: usize = 128 * 1024;

/// A task-local draft. Tools never mutate the live editor or the saved resume.
pub struct ResumeTools {
    document: String,
    draft: String,
    proposed: bool,
    build: Option<crate::sandbox::CompiledDraft>,
    directory: Option<PathBuf>,
}

impl ResumeTools {
    pub fn new(document: String, source: String) -> Self {
        Self {
            document,
            draft: source,
            proposed: false,
            build: None,
            directory: None,
        }
    }

    fn stage(&mut self, draft: String) -> Result<Value> {
        ensure!(
            !draft.trim().is_empty(),
            "The proposed resume cannot be empty"
        );
        ensure!(
            draft.len() <= MAX_DRAFT_BYTES,
            "The proposed resume is too large"
        );

        self.draft = draft;
        self.proposed = true;
        // Every edit invalidates the PDF, including targeted replacements.
        self.build = None;

        Ok(json!({"staged": true, "saved": false, "bytes": self.draft.len()}))
    }

    pub fn take_build(&mut self) -> Option<crate::sandbox::CompiledDraft> {
        self.build.take()
    }

    pub fn in_directory(mut self, directory: PathBuf) -> Self {
        self.directory = Some(directory);
        self
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArguments {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposedEdit {
    tex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextReplacement {
    old_text: String,
    new_text: String,
}

impl ToolSet for ResumeTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "replace_text".into(),
                description: "Stage a targeted edit by replacing one exact, unique old_text with new_text. \
                    Include enough surrounding text to identify a single match. Prefer this for small edits; \
                    unchanged text is preserved. Call compile_resume after editing.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "old_text": {"type": "string", "minLength": 1, "maxLength": MAX_DRAFT_BYTES},
                        "new_text": {"type": "string", "maxLength": MAX_DRAFT_BYTES}
                    },
                    "required": ["old_text", "new_text"],
                    "additionalProperties": false
                }),
            },
            ToolDefinition {
                name: "compile_resume".into(),
                description: "Compile the current draft offline in an OS sandbox. Errors are returned so you can revise \
                    and retry. Must succeed after the last edit before finishing. Does not publish the PDF.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
            ToolDefinition {
                name: "read_resume".into(),
                description: "Read the task's current resume draft, including unsaved text and any staged edits. \
                    This tool does not read other files.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": [],
                    "additionalProperties": false
                }),
            },
            ToolDefinition {
                name: "propose_edit".into(),
                description: "Stage a COMPLETE revised TeX document in memory. Does not change the editor or save files. \
                    Preserve the user's facts.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "tex": {"type": "string", "maxLength": MAX_DRAFT_BYTES}
                    },
                    "required": ["tex"],
                    "additionalProperties": false
                }),
            },
        ]
    }

    fn execute(&mut self, name: &str, arguments: &Value, cancelled: &AtomicBool) -> Result<Value> {
        ensure!(!cancelled.load(Ordering::Relaxed), "Assistant cancelled");

        match name {
            "replace_text" => {
                let edit: TextReplacement =
                    serde_json::from_value(arguments.clone()).map_err(|_| {
                        anyhow!("replace_text requires only old_text and new_text strings")
                    })?;
                ensure!(!edit.old_text.is_empty(), "old_text cannot be empty");
                ensure!(
                    edit.old_text.len() <= MAX_DRAFT_BYTES
                        && edit.new_text.len() <= MAX_DRAFT_BYTES,
                    "The replacement is too large"
                );

                // Checking both ends also rejects overlapping matches, such as "aa" in "aaa".
                let first_match = self.draft.find(&edit.old_text);
                ensure!(
                    first_match.is_some() && first_match == self.draft.rfind(&edit.old_text),
                    "old_text must match exactly once in the current draft; include more context"
                );

                let draft = self.draft.replacen(&edit.old_text, &edit.new_text, 1);
                self.stage(draft)
            }
            "compile_resume" => {
                serde_json::from_value::<NoArguments>(arguments.clone())
                    .map_err(|_| anyhow!("compile_resume accepts no arguments"))?;

                self.build = None;
                let directory = self
                    .directory
                    .as_ref()
                    .ok_or_else(|| anyhow!("No resume folder selected"))?;

                let build = crate::sandbox::build(&self.draft, directory, cancelled)?;
                let result = json!({"compiled":true, "diagnostics":build.diagnostics});

                self.build = Some(build);

                Ok(result)
            }
            "read_resume" => {
                serde_json::from_value::<NoArguments>(arguments.clone())
                    .map_err(|_| anyhow!("read_resume accepts no arguments"))?;

                Ok(json!({
                    "document": self.document,
                    "tex": self.draft,
                    "has_proposal": self.proposed
                }))
            }
            "propose_edit" => {
                let edit: ProposedEdit = serde_json::from_value(arguments.clone())
                    .map_err(|_| anyhow!("propose_edit requires only a tex string"))?;

                self.stage(edit.tex)
            }
            _ => bail!("Unknown resume tool"),
        }
    }

    fn finish(&mut self, mut reply: AssistantReply) -> Result<AssistantReply> {
        if self.proposed {
            ensure!(
                self.build.is_some(),
                "Compile the staged draft successfully before completing the edit"
            );
            ensure!(
                reply
                    .proposed_tex
                    .as_ref()
                    .is_none_or(|tex| tex == &self.draft),
                "Final edit differs from the staged draft"
            );

            reply.proposed_tex = Some(self.draft.clone());
        } else {
            ensure!(
                reply.proposed_tex.is_none(),
                "Use replace_text or propose_edit before completing an edit"
            );
        }

        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use crate::{
        agent::{self, AgentTurn},
        tools::ToolCall,
    };

    #[test]
    #[ignore = "requires macOS, cached Tectonic, and Poppler"]
    fn batched_edit_reuses_validated_pdf_and_invalidates_it_on_revision() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let asset = directory.path().join("bullet.tex");
        std::fs::write(&asset, "Example bullet")?;
        let source = r"\documentclass{article}\begin{document}\input{bullet}\end{document}";
        let mut tools = ResumeTools::new("resume.tex".into(), "Original".into())
            .in_directory(directory.path().to_owned());
        let calls = vec![
            crate::tools::ToolCall {
                id: "edit".into(),
                name: "propose_edit".into(),
                arguments: json!({"tex": source}),
            },
            crate::tools::ToolCall {
                id: "compile".into(),
                name: "compile_resume".into(),
                arguments: json!({}),
            },
        ];
        let cancelled = AtomicBool::new(false);
        let definitions = tools.definitions();
        let results = crate::tools::execute_calls(&mut tools, &definitions, &calls, &cancelled)?;
        assert!(results.iter().all(|result| !result.is_error));

        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: Some("Different from the compiled source".into()),
                })
                .is_err()
        );

        let reply = tools.finish(AssistantReply {
            reply: "Changed bullet".into(),
            proposed_tex: None,
        })?;
        let build = tools
            .take_build()
            .expect("Successful compilation must retain its PDF");
        let pdf_path = build.pdf_path.clone();
        let bytes = std::fs::read(&pdf_path)?;
        std::fs::remove_file(&asset)?;

        // Recompiling now would fail: the preview must consume the validated artifact.
        let preview =
            crate::compiler::render_compiled(reply.proposed_tex.unwrap(), build, &cancelled)?;
        assert_eq!(preview.pages.page_count(), 1);
        assert_eq!(std::fs::read(&preview.pdf_path)?, bytes);
        drop(preview);
        assert!(!pdf_path.exists());

        std::fs::write(&asset, "Restored bullet")?;
        tools.execute("compile_resume", &json!({}), &cancelled)?;
        let previous_pdf = tools.build.as_ref().unwrap().pdf_path.clone();
        tools.execute(
            "propose_edit",
            &json!({"tex": "Invalid revision"}),
            &cancelled,
        )?;
        assert!(tools.take_build().is_none());
        assert!(!previous_pdf.exists());
        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: None
                })
                .is_err()
        );
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn call(id: &str, name: &str, arguments: Value) -> AgentTurn {
        AgentTurn::Tools {
            calls: vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments,
            }],
            continuation: Value::Null,
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires installed Tectonic and cached LaTeX packages"]
    fn agent_repairs_a_compile_error_without_changing_saved_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resume.tex");
        std::fs::write(&path, "saved source").unwrap();
        let mut tools = ResumeTools::new("resume.tex".into(), "unsaved source".into())
            .in_directory(directory.path().to_owned());

        let valid = r"\documentclass{article}\begin{document}Revised\end{document}";
        let broken = concat!(
            r"\documentclass{article}\begin{document}",
            r"\undefinedcommand\end{document}",
        );

        let result = agent::run(
            "Edit resume",
            &mut tools,
            &AtomicBool::new(false),
            |_, _, history, _| {
                Ok(match history.len() {
                    0 => call("read", "read_resume", json!({})),
                    1 => {
                        assert_eq!(history[0].results[0].output["tex"], "unsaved source");
                        call("bad", "propose_edit", json!({"tex":broken}))
                    }
                    2 => call("fail", "compile_resume", json!({})),
                    3 => {
                        assert!(history[2].results[0].is_error);
                        call("repair", "propose_edit", json!({"tex":valid}))
                    }
                    4 => call("success", "compile_resume", json!({})),
                    _ => {
                        assert!(!history[4].results[0].is_error, "{:?}", history[4]);
                        AgentTurn::Complete(AssistantReply {
                            reply: "Updated draft".into(),
                            proposed_tex: None,
                        })
                    }
                })
            },
        )
        .unwrap();

        assert_eq!(result.proposed_tex.as_deref(), Some(valid));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved source");

        tools
            .execute(
                "propose_edit",
                &json!({"tex":"another draft"}),
                &AtomicBool::new(false),
            )
            .unwrap();
        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: None
                })
                .is_err()
        );
    }

    #[test]
    fn tools_reject_file_selectors_unknown_fields_empty_and_oversized_edits() {
        let mut tools = ResumeTools::new("resume.tex".into(), "original".into());
        let cancelled = AtomicBool::new(false);

        for arguments in [json!({"path":"../../private"}), json!(null)] {
            assert!(
                tools
                    .execute("read_resume", &arguments, &cancelled)
                    .is_err()
            );
        }

        for arguments in [
            json!({"tex":""}),
            json!({"tex":"new", "path":"/tmp/other"}),
            json!({"tex":"x".repeat(MAX_DRAFT_BYTES+1)}),
        ] {
            assert!(
                tools
                    .execute("propose_edit", &arguments, &cancelled)
                    .is_err()
            );
        }

        assert_eq!(tools.draft, "original");
        assert!(tools.execute("shell", &json!({}), &cancelled).is_err());
        assert!(
            tools
                .execute(
                    "propose_edit",
                    &json!({"tex":"new"}),
                    &AtomicBool::new(true)
                )
                .is_err()
        );

        assert_eq!(tools.draft, "original");
    }

    #[test]
    fn targeted_edits_preserve_other_text_and_require_compilation() {
        let original = "Résumé: original bullet. Other content.";
        let mut tools = ResumeTools::new("resume.tex".into(), original.into());
        let cancelled = AtomicBool::new(false);

        tools
            .execute(
                "replace_text",
                &json!({"old_text": "original bullet", "new_text": "improved bullet"}),
                &cancelled,
            )
            .unwrap();

        assert_eq!(tools.draft, "Résumé: improved bullet. Other content.");
        assert!(tools.proposed);
        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: None,
                })
                .is_err()
        );

        tools
            .execute(
                "replace_text",
                &json!({"old_text": " Other content.", "new_text": ""}),
                &cancelled,
            )
            .unwrap();
        assert_eq!(tools.draft, "Résumé: improved bullet.");
    }

    #[test]
    fn rejected_replacements_leave_the_draft_unchanged() {
        let original = "aaa repeated repeated";
        let mut tools = ResumeTools::new("resume.tex".into(), original.into());
        let cancelled = AtomicBool::new(false);

        for arguments in [
            json!({"old_text": "", "new_text": "new"}),
            json!({"old_text": "missing", "new_text": "new"}),
            json!({"old_text": "repeated", "new_text": "new"}),
            json!({"old_text": "aa", "new_text": "new"}),
            json!({"old_text": original, "new_text": " "}),
            json!({"old_text": "aaa", "new_text": "x".repeat(MAX_DRAFT_BYTES)}),
            json!({"old_text": "aaa", "new_text": "new", "path": "/other"}),
            json!({"old_text": "aaa"}),
        ] {
            assert!(
                tools
                    .execute("replace_text", &arguments, &cancelled)
                    .is_err()
            );
            assert_eq!(tools.draft, original);
            assert!(!tools.proposed);
        }

        assert!(
            tools
                .execute(
                    "replace_text",
                    &json!({"old_text": "aaa", "new_text": "new"}),
                    &AtomicBool::new(true),
                )
                .is_err()
        );
        assert_eq!(tools.draft, original);
    }

    #[test]
    #[ignore = "requires macOS and cached Tectonic"]
    fn targeted_edit_invalidates_the_compiled_pdf() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = r"\documentclass{article}\begin{document}Original\end{document}";
        let mut tools = ResumeTools::new("resume.tex".into(), source.into())
            .in_directory(directory.path().to_owned());
        let cancelled = AtomicBool::new(false);
        tools.execute("compile_resume", &json!({}), &cancelled)?;
        let previous_pdf = tools.build.as_ref().unwrap().pdf_path.clone();

        assert!(
            tools
                .execute(
                    "replace_text",
                    &json!({"old_text": "Missing", "new_text": "Revised"}),
                    &cancelled,
                )
                .is_err()
        );
        assert!(tools.build.is_some());
        assert!(previous_pdf.exists());

        tools.execute(
            "replace_text",
            &json!({"old_text": "Original", "new_text": "Revised"}),
            &cancelled,
        )?;

        assert!(tools.build.is_none());
        assert!(!previous_pdf.exists());
        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: None,
                })
                .is_err()
        );

        tools.execute("compile_resume", &json!({}), &cancelled)?;
        let reply = tools.finish(AssistantReply {
            reply: "Revised".into(),
            proposed_tex: None,
        })?;
        assert_eq!(
            reply.proposed_tex,
            Some(source.replace("Original", "Revised"))
        );
        Ok(())
    }

    #[test]
    fn completion_cannot_bypass_the_staged_proposal() {
        let mut tools = ResumeTools::new("resume.tex".into(), "original".into());
        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: Some("unstaged".into())
                })
                .is_err()
        );

        tools
            .execute(
                "propose_edit",
                &json!({"tex":"staged"}),
                &AtomicBool::new(false),
            )
            .unwrap();

        assert!(
            tools
                .finish(AssistantReply {
                    reply: "Done".into(),
                    proposed_tex: Some("different".into())
                })
                .is_err()
        );
    }
}
