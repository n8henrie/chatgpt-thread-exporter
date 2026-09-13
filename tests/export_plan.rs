use std::error::Error;
use std::io;

use chatgpt_thread_exporter::build_export_plan_json;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/export-input.json");

#[test]
fn exports_the_active_branch_and_artifacts() -> Result<(), Box<dyn Error>> {
    let output = build_export_plan_json(FIXTURE)?;
    let plan: Value = serde_json::from_str(&output)?;
    let markdown = string_field(&plan, "markdown")?;

    assert_eq!(plan["extraction"], "api");
    assert_eq!(
        plan["conversation_id"],
        "11111111-2222-3333-4444-555555555555"
    );
    assert!(markdown.contains("## User"));
    assert!(markdown.contains("## ChatGPT"));
    assert!(markdown.contains("report.pdf"));
    assert!(!markdown.contains('\u{e200}'));
    assert!(!output.contains("secret-value"));

    let branches = array_field(&plan, "branches")?;
    assert_eq!(branches.len(), 1);
    assert!(string_field(&branches[0], "markdown")?.contains("Alternate response."));

    let artifacts = array_field(&plan, "artifacts")?;
    assert_eq!(artifacts.len(), 2);
    assert!(artifacts.iter().any(|artifact| {
        artifact["direction"] == "submitted"
            && artifact["relative_path"] == "submitted/001-submitted-photo.png"
    }));
    let received = artifacts
        .iter()
        .find(|artifact| artifact["direction"] == "received")
        .ok_or_else(|| io::Error::other("missing received artifact"))?;
    assert_eq!(received["relative_path"], "received/001-report.pdf");
    assert_eq!(received["resolution"][0]["kind"], "api");
    assert_eq!(
        received["resolution"][0]["path"],
        concat!(
            "/backend-api/conversation/11111111-2222-3333-4444-555555555555/",
            "interpreter/download?message_id=assistant-message-main",
            "&sandbox_path=%2Fmnt%2Fdata%2Freport.pdf"
        )
    );

    Ok(())
}

#[test]
fn sanitizes_reserved_windows_filenames() -> Result<(), Box<dyn Error>> {
    let mut input: Value = serde_json::from_str(FIXTURE)?;
    input["structured_conversation"]["mapping"]["user-1"]["message"]["metadata"]
        ["attachments"][0]["file_name"] = Value::String(String::from("CON.txt"));

    let output = build_export_plan_json(&serde_json::to_string(&input)?)?;
    let plan: Value = serde_json::from_str(&output)?;
    let artifacts = array_field(&plan, "artifacts")?;

    assert!(artifacts.iter().any(|artifact| {
        artifact["direction"] == "submitted"
            && artifact["relative_path"] == "submitted/001-_CON.txt"
    }));

    Ok(())
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, io::Error> {
    value[field]
        .as_str()
        .ok_or_else(|| io::Error::other(format!("missing string field {field}")))
}

fn array_field<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], io::Error> {
    value[field]
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| io::Error::other(format!("missing array field {field}")))
}
