use super::*;
use anyhow::ensure;
use ratatui::{Terminal, backend::TestBackend};
use secrecy::ExposeSecret;
use std::process::Command;
use std::{cell::RefCell, rc::Rc};
use std::{
    thread,
    time::{Duration, Instant},
};

struct TestStore {
    saved: Rc<RefCell<Option<String>>>,
    fail: bool,
}
impl SecretStore for TestStore {
    fn get(&self, _: Provider) -> Result<Option<SecretString>> {
        Ok(self
            .saved
            .borrow()
            .as_ref()
            .map(|key| SecretString::from(key.clone())))
    }
    fn set(&self, _: Provider, key: &SecretString) -> Result<()> {
        ensure!(!self.fail, "Credential store is locked");
        *self.saved.borrow_mut() = Some(key.expose_secret().into());
        Ok(())
    }
    fn delete(&self, _: Provider) -> Result<()> {
        Ok(())
    }
}
fn dialog(fail: bool) -> (tempfile::TempDir, LoginDialog, Rc<RefCell<Option<String>>>) {
    let directory = tempfile::tempdir().unwrap();
    let saved = Rc::new(RefCell::new(None));
    let mut dialog = LoginDialog::new();
    dialog.model_loader = |_, _, _| Ok(vec!["test-model-new".into(), "test-model-old".into()]);
    dialog.preferences = Some(directory.path().join("provider.json"));
    dialog.store = Box::new(TestStore {
        saved: Rc::clone(&saved),
        fail,
    });
    (directory, dialog, saved)
}
fn enter(dialog: &mut LoginDialog) -> LoginResult {
    let deadline = Instant::now() + Duration::from_secs(2);
    while dialog.step == Step::LoadingModels && Instant::now() < deadline {
        dialog.poll();
        thread::yield_now();
    }
    dialog.handle(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Rect::new(0, 0, 80, 20),
    )
}

#[test]
fn api_key_flow_masks_secrets_and_persists_only_preferences_for_both_providers() {
    for provider in [Provider::OpenAi, Provider::Anthropic] {
        let (_directory, mut dialog, saved) = dialog(false);
        dialog.selection = usize::from(provider == Provider::Anthropic);
        enter(&mut dialog);
        dialog.selection = usize::from(provider == Provider::OpenAi);
        enter(&mut dialog);
        assert!(dialog.step == Step::Key);
        dialog.handle(
            Event::Paste("test-secret-never-render".into()),
            Rect::default(),
        );
        let mut terminal = Terminal::new(TestBackend::new(90, 20)).unwrap();
        terminal
            .draw(|frame| dialog.draw(frame, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(!text.contains("test-secret"));
        enter(&mut dialog);
        assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
        assert!(dialog.key.is_empty());
        assert_eq!(saved.borrow().as_deref(), Some("test-secret-never-render"));
        let contents = std::fs::read_to_string(dialog.preferences.as_ref().unwrap()).unwrap();
        assert!(!contents.contains("test-secret"));
        assert_eq!(
            Profile::load_from(dialog.preferences.as_ref().unwrap())
                .unwrap()
                .provider,
            provider
        );
    }
}

#[test]
fn existing_saved_keys_can_be_selected_without_reentering_them() {
    let (_directory, mut dialog, saved) = dialog(false);
    *saved.borrow_mut() = Some("saved-test-secret".into());
    dialog.provider = Provider::Anthropic;
    dialog.step = Step::Model;
    dialog.models = vec!["test-model".into()];
    dialog.reuse_saved_key = true;
    assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
    assert!(dialog.key.is_empty());
    assert_eq!(saved.borrow().as_deref(), Some("saved-test-secret"));
}

#[test]
fn failed_storage_and_invalid_paste_do_not_change_the_profile() {
    let (_directory, mut dialog, saved) = dialog(true);
    dialog.provider = Provider::Anthropic;
    dialog.model = dialog.provider.default_model().into();
    dialog.step = Step::Key;
    dialog.handle(Event::Paste("secret\nsecond-line".into()), Rect::default());
    assert!(dialog.key.is_empty());
    dialog.handle(Event::Paste("test-secret".into()), Rect::default());
    enter(&mut dialog);
    assert!(matches!(enter(&mut dialog), LoginResult::Pending));
    assert!(dialog.error.contains("locked"));
    assert!(saved.borrow().is_none());
    assert!(!dialog.preferences.as_ref().unwrap().exists());
}

#[cfg(unix)]
#[test]
fn existing_account_success_failure_and_timeout_use_the_same_dialog() {
    for (script, expected_success) in [
        ("exit 0", true),
        ("exit 1", false),
        ("exec sleep 10", false),
    ] {
        let (_directory, mut dialog, _saved) = dialog(false);
        let mut child = Command::new("sh").args(["-c", script]).spawn().unwrap();

        // Wait for completed stubs before exercising the same timeout path as a running child.
        if script.starts_with("exit") {
            child.wait().unwrap();
        }
        dialog.process = Some(LoginProcess::for_test(
            child,
            Instant::now() - texo::auth::CODEX_STATUS_TIMEOUT,
        ));
        dialog.step = Step::Waiting;

        let profile = dialog.poll();
        assert_eq!(profile.is_some(), expected_success);
        if expected_success {
            assert_eq!(profile.unwrap().provider, Provider::Codex);
            assert_eq!(
                Profile::load_from(dialog.preferences.as_ref().unwrap())
                    .unwrap()
                    .provider,
                Provider::Codex
            );
        } else {
            assert!(!dialog.error.is_empty());
            assert!(dialog.step == Step::Method);
            assert!(!dialog.preferences.as_ref().unwrap().exists());
        }
    }
}

#[test]
fn missing_saved_key_prompts_for_a_key_before_fetching_models() {
    for provider in [Provider::OpenAi, Provider::Anthropic] {
        let (_directory, mut dialog, _) = dialog(false);
        dialog.provider = provider;
        dialog.step = Step::Method;
        dialog.selection = if provider == Provider::OpenAi { 2 } else { 1 };
        enter(&mut dialog);
        assert!(dialog.step == Step::Key);
        assert!(dialog.model_job.is_none());
        assert!(!dialog.reuse_saved_key);
        dialog.handle(Event::Paste("new-test-key".into()), Rect::default());
        enter(&mut dialog);
        assert!(dialog.step == Step::LoadingModels);
        assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
    }
}

#[test]
fn saved_key_is_passed_to_discovery_before_model_selection() {
    let (_directory, mut dialog, saved) = dialog(false);
    *saved.borrow_mut() = Some("saved-discovery-key".into());
    dialog.model_loader = |provider, key, _| {
        assert_eq!(provider, Provider::OpenAi);
        assert_eq!(key.expose_secret(), "saved-discovery-key");
        Ok(vec!["gpt-live-catalog".into()])
    };
    dialog.step = Step::Method;
    dialog.selection = 2;
    enter(&mut dialog);
    assert!(dialog.step == Step::LoadingModels);
    let LoginResult::Connected(profile) = enter(&mut dialog) else {
        panic!("expected selected model")
    };
    assert_eq!(profile.model, "gpt-live-catalog");
}

#[test]
fn model_switch_preselects_current_model_and_preserves_credentials() {
    for provider in [Provider::OpenAi, Provider::Anthropic] {
        let (_directory, mut dialog, saved) = dialog(false);
        *saved.borrow_mut() = Some("existing-secret".into());
        dialog.provider = provider;
        dialog.model_only = true;
        dialog.reuse_saved_key = true;
        dialog.model = "test-model-old".into();
        dialog.start_model_discovery().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while dialog.step == Step::LoadingModels && Instant::now() < deadline {
            dialog.poll();
            thread::yield_now();
        }
        assert_eq!(dialog.selection, 1);
        assert!(dialog.is_model_selection());
        dialog
            .handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
            .unwrap();
        let LoginResult::Connected(profile) = enter(&mut dialog) else {
            panic!("expected selected model")
        };
        assert_eq!(profile.provider, provider);
        assert_eq!(profile.model, "test-model-new");
        assert_eq!(profile.key_source, KeySource::Saved);
        assert_eq!(saved.borrow().as_deref(), Some("existing-secret"));
    }
}

#[test]
fn model_command_rejects_disconnected_profiles() {
    assert!(LoginDialog::for_model(&Profile::new(Provider::Disconnected)).is_err());
}

#[cfg(unix)]
#[test]
fn successful_codex_login_can_immediately_select_and_persist_a_model() {
    let (_directory, mut login, saved) = dialog(false);
    let mut child = Command::new("sh").args(["-c", "exit 0"]).spawn().unwrap();
    child.wait().unwrap();
    login.process = Some(LoginProcess::for_test(child, Instant::now()));
    login.step = Step::Waiting;
    let profile = login.poll().expect("Codex login should succeed");
    assert_eq!(profile.provider, Provider::Codex);
    let loaded = Profile::load_from(login.preferences.as_ref().unwrap()).unwrap();
    let mut picker = LoginDialog::for_model(&loaded).expect("/model after login should open");
    picker.preferences = login.preferences.clone();
    assert!(picker.step == Step::Model);
    assert!(picker.model_job.is_none());
    picker
        .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
        .unwrap();
    let expected = picker.models[picker.selection].clone();
    let LoginResult::Connected(selected) = enter(&mut picker) else {
        panic!("expected selected model")
    };
    assert_eq!(selected.model, expected);
    let persisted = Profile::load_from(login.preferences.as_ref().unwrap()).unwrap();
    assert_eq!(persisted.provider, Provider::Codex);
    assert_eq!(persisted.model, expected);
    let reopened = LoginDialog::for_model(&persisted).unwrap();
    assert_eq!(reopened.models[reopened.selection], expected);
    assert!(saved.borrow().is_none());
}

#[test]
fn model_picker_scrolls_and_saves_the_selected_live_model() {
    let (_directory, mut dialog, _) = dialog(false);
    dialog.step = Step::Model;
    dialog.key.push_str("test-secret");
    dialog.models = (0..40).map(|i| format!("gpt-live-{i}")).collect();
    for _ in 0..25 {
        dialog
            .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
    }
    dialog
        .handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    let mut terminal = Terminal::new(TestBackend::new(70, 12)).unwrap();
    terminal
        .draw(|frame| dialog.draw(frame, frame.area()))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("› gpt-live-24"));
    let LoginResult::Connected(profile) = enter(&mut dialog) else {
        panic!("model not saved")
    };
    assert_eq!(profile.model, "gpt-live-24");
}

#[test]
fn model_discovery_errors_allow_retry_without_saving_credentials() {
    let (_directory, mut dialog, saved) = dialog(false);
    dialog.key.push_str("test-secret");
    dialog.model_loader = |_, _, _| Err(anyhow!("Network unavailable"));
    dialog.start_model_discovery().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while dialog.step == Step::LoadingModels && Instant::now() < deadline {
        dialog.poll();
        thread::yield_now();
    }
    assert!(dialog.error.contains("Network unavailable"));
    assert!(saved.borrow().is_none());
    assert!(!dialog.preferences.as_ref().unwrap().exists());
    dialog.model_loader = |_, _, _| Ok(vec!["gpt-new-from-api".into()]);
    enter(&mut dialog);
    assert!(matches!(enter(&mut dialog), LoginResult::Connected(_)));
}

#[test]
fn cancelling_and_tiny_terminals_are_safe() {
    let (_directory, mut dialog, saved) = dialog(false);
    dialog.step = Step::Key;
    dialog.key.push_str("discard-secret");
    assert!(matches!(
        dialog.handle(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Rect::default()
        ),
        LoginResult::Cancelled
    ));
    assert!(dialog.key.is_empty());
    for (width, height) in [(1, 1), (20, 5)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| dialog.draw(frame, frame.area()))
            .unwrap();
    }
    assert!(saved.borrow().is_none());
}
