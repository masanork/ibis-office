// SPDX-License-Identifier: AGPL-3.0-only

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = serde_json::from_reader(std::io::stdin().lock())?;
    let result = ibis_doc_service::compare_presentation(&input)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
