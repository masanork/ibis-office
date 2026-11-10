// SPDX-License-Identifier: AGPL-3.0-only
use ibis_doc_model::{Command, Document, EditError, Presentation};

#[test]
fn readonly_document_rejects_changes_and_new_document_is_editable() {
    let mut doc = Document::new(Presentation::new());
    doc.set_read_only(true);
    let command = Command::SetThemeEffects { name: Some("test".into()) };
    assert!(matches!(doc.execute(vec![command.clone()]), Err(EditError::ReadOnly)));
    assert!(doc.presentation().theme_effects.is_none());
    let mut next = Document::new(Presentation::new());
    assert!(next.execute(vec![command]).is_ok());
}
