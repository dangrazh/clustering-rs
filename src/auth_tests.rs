use super::*;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde_json::{json, Value};

#[tokio::test]
async fn allowlist_identity_reload_and_validation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path().join("data"), 4).await?;
    let path = dir.path().join("users.json");
    std::fs::write(
        &path,
        r#"[{"email":" Alice@Example.invalid ","name":" Alice Reviewer "}]"#,
    )?;
    let config = AllowlistConfig {
        path: path.clone(),
        session_seconds: 3600,
        secure_cookie: true,
    };
    let auth = Auth::with_allowlist(store.clone(), config.clone()).await?;
    assert!(store.users().await?.is_empty());
    assert!(auth.email_login("unknown@example.invalid").await.is_err());
    let first = auth.email_login("ALICE@example.invalid ").await?;
    assert!(first.contains("; Secure;"));
    let a = auth.session(&first).await?;
    let second = auth.email_login("alice@example.invalid").await?;
    assert_eq!(a.user.id, auth.session(&second).await?.user.id);
    assert_eq!(a.user.name, "Alice Reviewer");
    std::fs::write(
        &path,
        r#"[{"email":"alice@example.invalid","name":"Updated Name"}]"#,
    )?;
    assert_eq!(auth.session(&first).await?.user.name, "Updated Name");
    assert_eq!(
        auth.session(&auth.email_login("alice@example.invalid").await?)
            .await?
            .user
            .id,
        a.user.id
    );
    std::fs::write(&path, "[]")?;
    assert!(auth.session(&first).await.is_err());
    for invalid in [
        r#"[{"email":"a@example.invalid","name":"A"},{"email":"A@example.invalid","name":"B"}]"#,
        r#"[{"email":"bad","name":"Name"}]"#,
        r#"[{"email":"a@example.invalid","name":" "}]"#,
        "not JSON",
    ] {
        std::fs::write(&path, invalid)?;
        assert!(Auth::with_allowlist(store.clone(), config.clone())
            .await
            .is_err());
        assert!(auth.session(&first).await.is_err());
    }
    std::fs::remove_file(&path)?;
    assert!(auth.email_login("alice@example.invalid").await.is_err());
    Ok(())
}

fn config() -> EntraConfig {
    EntraConfig {
        tenant: "00000000-0000-4000-8000-000000000001".into(),
        client: "00000000-0000-4000-8000-000000000002".into(),
        secret: "test-only".into(),
        redirect: "https://fixture.invalid/auth/callback".into(),
        allow_guests: false,
        session_seconds: 3600,
    }
}
fn claims(cfg: &EntraConfig) -> Value {
    json!({"tid":cfg.tenant,"aud":cfg.client,"iss":format!("https://login.microsoftonline.com/{}/v2.0",cfg.tenant),
        "oid":"00000000-0000-4000-8000-000000000003","nonce":"expected-nonce",
        "name":"Fixture Reviewer","email":"reviewer@example.invalid","acct":0,"nbf":now()-10,"exp":now()+300})
}
fn validate(value: &Value, cfg: &EntraConfig) -> Result<(Claims, String)> {
    let private = EncodingKey::from_rsa_pem(include_bytes!("auth_fixtures/test-only-private.pem"))?;
    let public = DecodingKey::from_rsa_pem(include_bytes!("auth_fixtures/test-only-public.pem"))?;
    validate_identity(
        &encode(&Header::new(Algorithm::RS256), value, &private)?,
        &public,
        cfg,
        "expected-nonce",
    )
}
#[test]
fn signed_identity_requires_company_claims_and_valid_protocol_values() {
    let cfg = config();
    let good = claims(&cfg);
    assert!(validate(&good, &cfg).is_ok());
    for (field, bad) in [
        ("tid", json!(id())),
        ("aud", json!(id())),
        ("iss", json!("https://attacker.invalid")),
        ("nonce", json!("wrong")),
        ("oid", json!("not-an-object-id")),
        ("exp", json!(now() - 120)),
        ("nbf", json!(now() + 120)),
        ("acct", json!(1)),
        ("name", json!("")),
        ("email", json!("invalid")),
    ] {
        let mut value = good.clone();
        value[field] = bad;
        assert!(validate(&value, &cfg).is_err(), "accepted invalid {field}");
    }
    for field in ["exp", "nbf", "aud", "iss", "acct"] {
        let mut value = good.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(validate(&value, &cfg).is_err(), "accepted missing {field}");
    }
}
#[tokio::test]
async fn sessions_are_opaque_revocable_and_independent_of_email_changes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = Store::open(dir.path(), 4).await?;
    let user = store
        .user("test:stable-id", "Alice", "alice@example.invalid")
        .await?;
    let auth = Auth::new(store.clone(), None)?;
    assert!(auth.login().is_err());
    let token = auth.issue(&user, 300).await?;
    let cookies = format!("app_session={token}");
    let session = auth.session(&cookies).await?;
    assert_eq!(session.user.id, user.id);
    assert!(!session.csrf.is_empty());
    let updated = store
        .user("test:stable-id", "Alice New", "new@example.invalid")
        .await?;
    assert_eq!(updated.id, user.id);
    assert_eq!(
        auth.session(&cookies).await?.user.email,
        "new@example.invalid"
    );
    assert!(auth.session("app_session=forged").await.is_err());
    let expired = auth.issue(&user, -1).await?;
    assert!(auth
        .session(&format!("app_session={expired}"))
        .await
        .is_err());
    auth.logout(&cookies).await?;
    assert!(auth.session(&cookies).await.is_err());
    Ok(())
}
