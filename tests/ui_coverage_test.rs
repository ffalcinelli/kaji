use kaji::utils::ui::{DialoguerUi, MockUi, Ui};
use std::sync::Mutex;

#[test]
fn test_dialoguer_ui_prints() {
    let ui = DialoguerUi::new();
    ui.print_info("info");
    ui.print_success("success");
    ui.print_error("error");
    ui.print_warn("warn");
}

#[test]
fn test_dialoguer_ui_with_term_branches() {
    let term = console::Term::buffered_stdout();
    let _ui = DialoguerUi::with_term(term);
    // Since we cannot easily mock the console read buffer in a way that
    // dialoguer 0.12 expects without a real TTY, we at least ensure
    // that we can instantiate it and hit the with_term constructors.
    // Further coverage of interact_on requires specialized mock terminals.
}

#[test]
fn test_mock_ui_password() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec!["secret123".to_string()]),
    };
    let pass = ui.password("Prompt", None).unwrap();
    assert_eq!(pass, "secret123");
}

#[test]
fn test_mock_ui_password_with_confirmation() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec!["secret123".to_string()]),
    };
    let pass = ui.password("Prompt", Some("Confirm Prompt")).unwrap();
    assert_eq!(pass, "secret123");
}

#[test]
fn test_mock_ui_input_success() {
    let ui = MockUi {
        inputs: Mutex::new(vec!["input1".to_string(), "input2".to_string()]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };
    assert_eq!(ui.input("p1", None, false).unwrap(), "input1");
    assert_eq!(ui.input("p2", Some("def".into()), true).unwrap(), "input2");
}

#[test]
fn test_mock_ui_confirm_success() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![true, false]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };
    assert!(ui.confirm("p1", true).unwrap());
    assert!(!ui.confirm("p2", false).unwrap());
}

#[test]
fn test_mock_ui_select_success() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![0, 1]),
        passwords: Mutex::new(vec![]),
    };
    assert_eq!(ui.select("p1", &["a", "b"], 0).unwrap(), 0);
    assert_eq!(ui.select("p2", &["a", "b"], 1).unwrap(), 1);
}

#[test]
fn test_mock_ui_errors() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };
    assert!(ui.input("p", None, false).is_err());
    assert!(ui.confirm("p", true).is_err());
    assert!(ui.select("p", &["a"], 0).is_err());
    assert!(ui.password("p", None).is_err());
}

#[test]
fn test_mock_ui_prints_do_nothing() {
    let ui = MockUi {
        inputs: Mutex::new(vec![]),
        confirms: Mutex::new(vec![]),
        selects: Mutex::new(vec![]),
        passwords: Mutex::new(vec![]),
    };
    ui.print_info("info");
    ui.print_success("success");
    ui.print_error("error");
    ui.print_warn("warn");
}
