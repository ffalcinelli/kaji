use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

#[cfg(test)]
mod common;
use common::start_mock_server;

#[test]
fn test_help() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: kaji"))
        .stdout(predicate::str::contains("Commands:"));
}

#[test]
fn test_version() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("kaji"));
}

#[test]
fn test_no_args() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.env_clear()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage: kaji [OPTIONS] <COMMAND>"));
}

#[test]
fn test_invalid_command() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("invalid")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unrecognized subcommand 'invalid'",
        ));
}

#[test]
fn test_validate_command() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg("http://localhost:8080")
        .arg("validate")
        .arg("--workspace")
        .arg(workspace)
        .assert()
        .success();
}

#[test]
fn test_validate_help() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("validate")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Validate the local Keycloak configuration files",
        ));
}

#[test]
fn test_inspect_help() {
    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("inspect")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Inspect the current Keycloak configuration and dump to files",
        ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_inspect_command() {
    let mock_url = start_mock_server().await;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg(&mock_url)
        .arg("--user")
        .arg("admin")
        .env("KEYCLOAK_PASSWORD", "admin")
        .arg("inspect")
        .arg("--workspace")
        .arg(&workspace)
        .arg("--yes")
        .assert()
        .success();

    assert!(workspace.join("test-realm").exists());
    assert!(workspace.join("test-realm").join("realm.yaml").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_apply_command() {
    let mock_url = start_mock_server().await;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let realm_dir = workspace.join("test-realm");
    fs::create_dir_all(&realm_dir).unwrap();

    fs::write(
        realm_dir.join("realm.yaml"),
        "realm: test-realm\nenabled: true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg(&mock_url)
        .arg("--client-id")
        .arg("admin-cli")
        .env("KEYCLOAK_CLIENT_SECRET", "secret")
        .arg("apply")
        .arg("--workspace")
        .arg(&workspace)
        .arg("--yes")
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_plan_command() {
    let mock_url = start_mock_server().await;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let realm_dir = workspace.join("test-realm");
    fs::create_dir_all(&realm_dir).unwrap();

    fs::write(
        realm_dir.join("realm.yaml"),
        "realm: test-realm\nenabled: true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg(&mock_url)
        .arg("--user")
        .arg("admin")
        .env("KEYCLOAK_PASSWORD", "admin")
        .arg("plan")
        .arg("--workspace")
        .arg(&workspace)
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_clean_command() {
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let realm_dir = workspace.join("test-realm");
    fs::create_dir_all(&realm_dir).unwrap();

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg("http://localhost:8080")
        .arg("clean")
        .arg("--workspace")
        .arg(&workspace)
        .arg("--yes")
        .assert()
        .success();

    assert!(!realm_dir.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_drift_command() {
    let mock_url = start_mock_server().await;
    let temp = tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let realm_dir = workspace.join("test-realm");
    fs::create_dir_all(&realm_dir).unwrap();

    fs::write(
        realm_dir.join("realm.yaml"),
        "realm: test-realm\nenabled: true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("kaji").unwrap();
    cmd.arg("--server")
        .arg(&mock_url)
        .arg("--user")
        .arg("admin")
        .env("KEYCLOAK_PASSWORD", "admin")
        .arg("drift")
        .arg("--workspace")
        .arg(&workspace)
        .assert()
        .success();
}
