// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{BTreeMap, BTreeSet, CryptoInventory, Row, Value, array};
use std::collections::VecDeque;

#[derive(Default)]
pub(super) struct Graph {
    nodes: BTreeMap<String, (String, String)>,
    dependents: BTreeMap<String, Vec<String>>,
    providers: BTreeMap<String, Vec<String>>,
}

impl Graph {
    pub(super) fn describe(&mut self, node: &Value, kind: &str) {
        if let Some(reference) = node["bom-ref"].as_str().filter(|r| !r.is_empty()) {
            self.nodes.insert(
                reference.into(),
                (
                    node["name"].as_str().unwrap_or_default().into(),
                    kind.into(),
                ),
            );
        }
    }

    pub(super) fn services(&mut self, parent: &Value) -> Result<(), String> {
        for service in array(parent, "services", true)? {
            self.describe(service, "service");
            self.services(service)?;
        }
        Ok(())
    }

    pub(super) fn add(&mut self, source: &str, kind: &str, target: &str) {
        let index = if kind == "dependsOn" {
            &mut self.dependents
        } else {
            &mut self.providers
        };
        index.entry(target.into()).or_default().push(source.into());
    }

    fn node(&self, reference: &str) -> (&str, &str) {
        self.nodes
            .get(reference)
            .map_or(("", ""), |(name, kind)| (name.as_str(), kind.as_str()))
    }
}

pub(super) const SNAPSHOT_LABELS: [&str; 2] = ["Old CBOM", "New CBOM"];

pub(super) const DEPENDENCY_COLUMNS: &[&str] = &[
    "Snapshot",
    "Asset reference",
    "Asset",
    "Dependent reference",
    "Dependent",
    "Dependent type",
    "Hops",
    "dependsOn path (references)",
];
pub(super) const PROVIDER_COLUMNS: &[&str] = &[
    "Snapshot",
    "Asset reference",
    "Asset",
    "Provider reference",
    "Provider",
    "Provider type",
    "Relationship",
];

pub(super) const SYSTEM_COLUMNS: &[&str] = &[
    "System reference",
    "Old system",
    "New system",
    "Old type",
    "New type",
    "Old changed assets",
    "New changed assets",
];

/// Summarize the same declared paths as the detail sheet, keyed by BOM reference.
pub(super) fn systems(paths: &[Row], old: &Graph, new: &Graph) -> Vec<Row> {
    let mut systems: BTreeMap<&str, [BTreeSet<(&str, &str)>; 2]> = BTreeMap::new();
    for path in paths {
        let (asset_ref, asset, system_ref) = (path[1].as_str(), path[2].as_str(), path[3].as_str());
        let (_, old_type) = old.node(system_ref);
        let (_, new_type) = new.node(system_ref);
        if ![old_type, new_type]
            .iter()
            .any(|kind| matches!(*kind, "application" | "service"))
        {
            continue;
        }
        let snapshot = usize::from(path[0] == SNAPSHOT_LABELS[1]);
        systems.entry(system_ref).or_default()[snapshot].insert((asset_ref, asset));
    }

    systems
        .into_iter()
        .map(|(reference, assets)| {
            let (old_name, old_type) = old.node(reference);
            let (new_name, new_type) = new.node(reference);
            let [old_assets, new_assets] = assets.map(|items| {
                items
                    .into_iter()
                    .map(|(reference, name)| {
                        if name.is_empty() {
                            reference.to_owned()
                        } else {
                            format!("{name} [{reference}]")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" · ")
            });
            vec![
                reference.into(),
                old_name.into(),
                new_name.into(),
                old_type.into(),
                new_type.into(),
                old_assets,
                new_assets,
            ]
        })
        .collect()
}

pub(super) fn rows(
    snapshots: [(&str, &CryptoInventory); 2],
    changed: &BTreeSet<String>,
) -> (Vec<Row>, Vec<Row>) {
    let mut dependent_rows = Vec::new();
    let mut provider_rows = Vec::new();
    for (label, inventory) in snapshots {
        let graph = &inventory.graph;
        for asset in inventory
            .assets
            .iter()
            .filter(|row| changed.contains(&row[2]))
        {
            let (reference, name) = (asset[2].as_str(), asset[0].as_str());
            let prefix = || vec![label.into(), reference.into(), name.into()];
            let mut visited = BTreeMap::from([(reference, (reference, 0usize))]);
            let mut pending = VecDeque::from([reference]);
            while let Some(target) = pending.pop_front() {
                let hops = visited[target].1 + 1;
                for source in graph
                    .dependents
                    .get(target)
                    .into_iter()
                    .flatten()
                    .map(String::as_str)
                {
                    let std::collections::btree_map::Entry::Vacant(entry) = visited.entry(source)
                    else {
                        continue;
                    };
                    entry.insert((target, hops));
                    pending.push_back(source);
                    let (source_name, kind) = graph.node(source);
                    if kind == "cryptographic-asset" {
                        continue;
                    }
                    let mut path = vec![source];
                    while *path.last().unwrap() != reference {
                        path.push(visited[path.last().unwrap()].0);
                    }
                    let mut row = prefix();
                    row.extend([
                        source.into(),
                        source_name.into(),
                        kind.into(),
                        hops.to_string(),
                        path.join(" → "),
                    ]);
                    dependent_rows.push(row);
                }
            }
            for provider in graph.providers.get(reference).into_iter().flatten() {
                let (provider_name, kind) = graph.node(provider);
                let mut row = prefix();
                row.extend([
                    provider.clone(),
                    provider_name.into(),
                    kind.into(),
                    "provides".into(),
                ]);
                provider_rows.push(row);
            }
        }
    }
    (dependent_rows, provider_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn traverses_non_crypto_nodes_and_cycles_without_following_providers() {
        let mut graph = Graph::default();
        for (reference, kind) in [
            ("system", "service"),
            ("bridge", "library"),
            ("provider", "library"),
            ("unrelated", "application"),
        ] {
            graph.describe(&json!({"bom-ref": reference, "name": reference}), kind);
        }
        graph.add("bridge", "dependsOn", "asset");
        graph.add("system", "dependsOn", "bridge");
        graph.add("bridge", "dependsOn", "system");
        graph.add("provider", "provides", "asset");
        graph.add("unrelated", "dependsOn", "provider");
        let inventory = CryptoInventory {
            assets: vec![vec!["Algorithm".into(), "algorithm".into(), "asset".into()]],
            edges: Vec::new(),
            graph,
        };
        let (dependents, providers) = rows(
            [("Old", &inventory), ("New", &inventory)],
            &BTreeSet::from(["asset".into()]),
        );
        assert_eq!(dependents.len(), 4);
        assert_eq!(dependents[1][3], "system");
        assert_eq!(
            dependents[1][5..],
            ["service", "2", "system → bridge → asset"]
        );
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0][3], "provider");

        let new = Graph::default();
        let summary = systems(&dependents[..2], &inventory.graph, &new);
        assert_eq!(
            summary,
            [vec![
                "system",
                "system",
                "",
                "service",
                "",
                "Algorithm [asset]",
                "",
            ]]
        );
    }
}
