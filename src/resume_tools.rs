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
    compiled: bool,
    directory: Option<PathBuf>,
}

impl ResumeTools {
    pub fn new(document: String, source: String) -> Self {
        Self {
            document,
            draft: source,
            proposed: false,
            compiled: false,
            directory: None,
        }
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

impl ToolSet for ResumeTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "compile_resume".into(),
                description: "Compile the current draft offline in an OS sandbox. Errors are returned so you can revise \
                    and retry. Must succeed after the last propose_edit before finishing. Does not publish the PDF.".into(),
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
            "compile_resume" => {
                serde_json::from_value::<NoArguments>(arguments.clone())
                    .map_err(|_| anyhow!("compile_resume accepts no arguments"))?;

                self.compiled = false;
                let directory = self
                    .directory
                    .as_ref()
                    .ok_or_else(|| anyhow!("No resume folder selected"))?;

                let diagnostics = crate::sandbox::compile(&self.draft, directory, cancelled)?;

                self.compiled = true;

                Ok(json!({"compiled":true, "diagnostics":diagnostics}))
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

                ensure!(
                    !edit.tex.trim().is_empty(),
                    "The proposed resume cannot be empty"
                );
                ensure!(
                    edit.tex.len() <= MAX_DRAFT_BYTES,
                    "The proposed resume is too large"
                );

                self.draft = edit.tex;
                self.proposed = true;

                self.compiled = false;

                Ok(json!({"staged":true, "saved":false, "bytes":self.draft.len()}))
            }
            _ => bail!("Unknown resume tool"),
        }
    }

    fn finish(&mut self, mut reply: AssistantReply) -> Result<AssistantReply> {
        if self.proposed {
            ensure!(
                self.compiled,
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
                "Use propose_edit before completing an edit"
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

        tools.compiled = true;
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
