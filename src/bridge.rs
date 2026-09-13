use serde_json::Value;

use crate::model::ResolvedFile;
use crate::security::trusted_https_url;

pub(crate) fn select_account_id(payload: &Value, workspace_id: Option<&str>) -> Option<String> {
    let record = payload.as_object()?;
    let accounts = record.get("accounts").and_then(Value::as_object);

    if let (Some(workspace_id), Some(accounts)) = (workspace_id, accounts)
        && let Some(identifier) = accounts
            .get(workspace_id)
            .and_then(account_id_from_entry)
    {
        return Some(identifier);
    }

    if let Some(identifier) = first_string(
        record,
        &[
            "account_id",
            "accountId",
            "default_account_id",
            "defaultAccountId",
        ],
    ) {
        return Some(identifier);
    }

    if let (Some(ordering), Some(accounts)) = (
        record.get("account_ordering").and_then(Value::as_array),
        accounts,
    ) {
        for key in ordering.iter().filter_map(Value::as_str) {
            if let Some(identifier) = accounts.get(key).and_then(account_id_from_entry) {
                return Some(identifier);
            }
        }
    }

    accounts.and_then(|accounts| {
        accounts
            .values()
            .find(|entry| {
                entry.as_object().is_some_and(|record| {
                    record
                        .get("is_default")
                        .and_then(Value::as_bool)
                        .is_some_and(std::convert::identity)
                        || record
                            .get("default")
                            .and_then(Value::as_bool)
                            .is_some_and(std::convert::identity)
                })
            })
            .and_then(account_id_from_entry)
            .or_else(|| accounts.values().find_map(account_id_from_entry))
    })
}

pub(crate) fn resolved_file_from_payload(payload: &Value) -> Result<ResolvedFile, String> {
    let Some(record) = payload.as_object() else {
        return Err(String::from("The ChatGPT file resolver returned an invalid response"));
    };
    if record
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status != "success")
    {
        return Err(String::from("The ChatGPT file resolver reported a failure"));
    }
    let Some(url) = first_string(record, &["download_url", "url"]) else {
        return Err(String::from(
            "The ChatGPT file resolver did not return a download URL",
        ));
    };
    if !trusted_https_url(&url) {
        return Err(String::from(
            "The ChatGPT file resolver returned an untrusted URL",
        ));
    }

    Ok(ResolvedFile {
        url,
        name: first_string(record, &["file_name", "filename", "name"]),
        mime_type: first_string(record, &["mime_type", "content_type"]),
        size_bytes: first_u64(record, &["file_size_bytes", "size_bytes", "size"]),
    })
}

fn account_id_from_entry(entry: &Value) -> Option<String> {
    let record = entry.as_object()?;
    let nested = record.get("account").and_then(Value::as_object);
    nested
        .and_then(|account| first_string(account, &["account_id", "accountId"]))
        .or_else(|| first_string(record, &["account_id", "accountId"]))
}

fn first_string(record: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        record
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn first_u64(record: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| record.get(*key).and_then(Value::as_u64))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{resolved_file_from_payload, select_account_id};

    #[test]
    fn selects_the_workspace_account() {
        let payload = json!({
            "accounts": {
                "workspace": {"account": {"account_id": "account-123"}},
                "other": {"account": {"account_id": "account-456"}}
            },
            "account_ordering": ["other"]
        });
        assert_eq!(
            select_account_id(&payload, Some("workspace")).as_deref(),
            Some("account-123")
        );
    }

    #[test]
    fn validates_resolver_urls() -> Result<(), String> {
        let file = resolved_file_from_payload(&json!({
            "download_url": "https://files.oaiusercontent.com/generated/report.pdf",
            "file_name": "report.pdf",
            "file_size_bytes": 42
        }))?;
        assert_eq!(file.name.as_deref(), Some("report.pdf"));
        assert_eq!(file.size_bytes, Some(42));
        Ok(())
    }
}
