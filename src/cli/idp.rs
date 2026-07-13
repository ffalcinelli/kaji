use crate::models::IdentityProviderRepresentation;
use crate::utils::ui::Ui;
use anyhow::{Context, Result};
use sanitize_filename::sanitize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

pub async fn create_idp_interactive(workspace_dir: &Path, ui: &dyn Ui) -> Result<()> {
    let realm = crate::cli::prompt_realm(workspace_dir, ui).await?;
    let alias = ui.input("IDP Alias (e.g. google, github)", None, false)?;
    let provider_id = ui.input(
        "Provider ID (e.g. oidc, saml, google)",
        Some(alias.clone()),
        false,
    )?;

    create_idp_yaml(workspace_dir, &realm, &alias, &provider_id).await?;

    ui.print_success(&format!(
        "Successfully generated YAML for Identity Provider '{}' in realm '{}'.",
        alias, realm
    ));
    Ok(())
}

pub async fn create_idp_yaml(
    workspace_dir: &Path,
    realm: &str,
    alias: &str,
    provider_id: &str,
) -> Result<()> {
    let idp = IdentityProviderRepresentation {
        internal_id: None,
        alias: Some(alias.to_string()),
        provider_id: Some(provider_id.to_string()),
        enabled: Some(true),
        update_profile_first_login_mode: Some("on".to_string()),
        trust_email: Some(false),
        store_token: Some(false),
        add_read_token_role_on_create: Some(false),
        authenticate_by_default: Some(false),
        link_only: Some(false),
        first_broker_login_flow_alias: Some("first broker login".to_string()),
        post_broker_login_flow_alias: None,
        display_name: Some(alias.to_string()),
        config: Some(HashMap::new()),
        extra: HashMap::new(),
    };

    let realm_dir = workspace_dir
        .join(sanitize(realm))
        .join("identity-providers");
    fs::create_dir_all(&realm_dir)
        .await
        .context("Failed to create identity-providers directory")?;

    let file_path = realm_dir.join(format!("{}.yaml", sanitize(alias)));
    let yaml = serde_yaml::to_string(&idp).context("Failed to serialize IDP to YAML")?;

    crate::utils::write_secure(&file_path, &yaml)
        .await
        .context("Failed to write IDP YAML file")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_idp_yaml() {
        let dir = tempdir().unwrap();
        let workspace_dir = dir.path();

        create_idp_yaml(workspace_dir, "master", "google", "google")
            .await
            .unwrap();

        let file_path = workspace_dir
            .join("master")
            .join("identity-providers")
            .join("google.yaml");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).await.unwrap();
        let idp: IdentityProviderRepresentation = serde_yaml::from_str(&content).unwrap();
        assert_eq!(idp.alias.as_deref(), Some("google"));
        assert_eq!(idp.provider_id.as_deref(), Some("google"));
        assert!(idp.enabled.unwrap());

        // Test with different provider_id
        create_idp_yaml(workspace_dir, "master", "my-idp", "oidc")
            .await
            .unwrap();
        let file_path2 = workspace_dir
            .join("master")
            .join("identity-providers")
            .join("my-idp.yaml");
        let content2 = fs::read_to_string(&file_path2).await.unwrap();
        let idp2: IdentityProviderRepresentation = serde_yaml::from_str(&content2).unwrap();
        assert_eq!(idp2.alias.as_deref(), Some("my-idp"));
        assert_eq!(idp2.provider_id.as_deref(), Some("oidc"));
    }
}
