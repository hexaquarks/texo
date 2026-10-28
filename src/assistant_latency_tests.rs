use super::*;

const BENCHMARK_REPETITIONS: usize = 3;
const SYNTHETIC_BULLETS: usize = 60;

fn synthetic_resume() -> String {
    let mut source = String::from(
        "\\documentclass{article}\n\\begin{document}\nDummy resume.\n\\begin{itemize}\n",
    );

    for index in 0..SYNTHETIC_BULLETS {
        source.push_str(&format!(
            concat!(
                "\\item Project {}: Built internal reporting tools, maintained automated tests, ",
                "and collaborated with colleagues to improve release quality.\n"
            ),
            index
        ));
    }

    source.push_str("\\end{itemize}\n\\end{document}\n");
    source
}

#[derive(Default)]
struct ModelTiming {
    turns: usize,
    elapsed: Duration,
}

impl ModelTiming {
    fn request(
        &mut self,
        profile: &Profile,
        prompt: &str,
        cancelled: &AtomicBool,
    ) -> Result<AgentTurn> {
        let started = Instant::now();
        let result = request(
            &mut codex_request_command(profile),
            prompt,
            cancelled,
            REQUEST_TIMEOUT,
        );
        self.elapsed += started.elapsed();
        self.turns += 1;

        if let Ok(AgentTurn::Tools { calls, .. }) = &result {
            for call in calls {
                eprintln!(
                    "BENCH tool={} argument_bytes={}",
                    call.name,
                    serde_json::to_vec(&call.arguments)?.len()
                );
            }
        }

        result
    }
}

fn direct_reply(
    profile: &Profile,
    source: &str,
    question: &str,
    timing: &mut ModelTiming,
    cancelled: &AtomicBool,
) -> Result<AssistantReply> {
    let prompt = format!(
        "You are a concise resume assistant. Use no tools. Answer the request using reply and proposed_tex; \
         return tool_calls=[]. For edits return the complete revised document in proposed_tex; \
         otherwise return null.\nRequest: {question}\nCurrent TeX:\n{source}"
    );

    match timing.request(profile, &prompt, cancelled)? {
        AgentTurn::Complete(reply) => Ok(reply),
        _ => bail!("Direct comparison unexpectedly requested tools"),
    }
}

fn texo_reply(
    profile: &Profile,
    prompt: &str,
    tools: &mut ResumeTools,
    timing: &mut ModelTiming,
    cancelled: &AtomicBool,
) -> Result<AssistantReply> {
    agent::run(
        prompt,
        tools,
        cancelled,
        |prompt, definitions, history, cancelled| {
            let prompt = codex_tool_prompt(&llm::TurnContext {
                prompt,
                tools: definitions,
                history,
            })?;

            timing.request(profile, &prompt, cancelled)
        },
    )
}

fn benchmark_sample(
    profile: &Profile,
    source: &str,
    question: &str,
    editing: bool,
    direct: bool,
) -> Result<()> {
    let folder = tempfile::tempdir()?;
    let cancelled = AtomicBool::new(false);
    let mut tools = ResumeTools::new("synthetic.tex".into(), source.into())
        .in_directory(folder.path().to_owned());
    let messages = [ConversationMessage {
        role: "user",
        content: question.into(),
    }];
    let prompt = build_prompt(source, &folder.path().join("synthetic.tex"), &messages)?;
    let mut timing = ModelTiming::default();

    let started = Instant::now();
    check_codex_login(&cancelled)?;
    let reply = if direct {
        direct_reply(profile, source, question, &mut timing, &cancelled)?
    } else {
        texo_reply(profile, &prompt, &mut tools, &mut timing, &cancelled)?
    };
    let response_time = started.elapsed();

    if editing {
        let expected = source.replace("Dummy resume.", "Example resume.");
        let actual = reply.proposed_tex.as_deref().context("Missing edit")?;
        eprintln!("BENCH exact_document={}", actual == expected);
        // Direct text responses sometimes omit the final newline. Still reject any changed content.
        ensure!(
            actual.trim_end() == expected.trim_end(),
            "Unexpected document change"
        );

        if direct {
            texo::sandbox::build(actual, folder.path(), &cancelled)?;
        } else {
            ensure!(actual == expected, "Texo changed unrelated whitespace");
            ensure!(tools.take_build().is_some(), "Missing validated PDF");
        }
    } else {
        ensure!(
            reply.proposed_tex.is_none(),
            "Advice must not edit the resume"
        );
    }

    eprintln!(
        "BENCH turns={} model_s={:.3} response_s={:.3} validated_s={:.3}",
        timing.turns,
        timing.elapsed.as_secs_f64(),
        response_time.as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

#[test]
#[ignore = "opt-in live Codex latency benchmark; uses account and cached Tectonic"]
fn live_agent_latency_comparison() -> Result<()> {
    if std::env::var("TEXO_RUN_LIVE_TESTS").as_deref() != Ok("1") {
        return Ok(());
    }

    let mut profile = Profile::new(Provider::Codex);
    profile.model = std::env::var("TEXO_BENCH_MODEL")
        .context("Set TEXO_BENCH_MODEL to pin the same model for both paths")?;
    profile.validate()?;
    let source = synthetic_resume();
    eprintln!(
        "BENCH model={} effort=low source_bytes={}",
        profile.model,
        source.len()
    );

    let cases = [
        (
            "advice",
            "In one sentence, should resume bullets start with action verbs? Do not edit the document.",
            false,
        ),
        (
            "edit",
            "Replace 'Dummy resume.' with 'Example resume.' and leave everything else unchanged.",
            true,
        ),
    ];

    for (label, question, editing) in cases {
        for repetition in 1..=BENCHMARK_REPETITIONS {
            // Alternate order to reduce systematic warm-cache and service-load bias.
            let order = if repetition % 2 == 0 {
                [true, false]
            } else {
                [false, true]
            };

            for direct in order {
                eprintln!(
                    "BENCH case={label} path={} run={repetition}",
                    if direct { "direct" } else { "texo" }
                );
                benchmark_sample(&profile, &source, question, editing, direct)?;
            }
        }
    }

    Ok(())
}
