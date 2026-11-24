# SPDX-License-Identifier: AGPL-3.0-only
"""Resolve an approved employment mapping from an OKF concept."""

from pathlib import Path

from review import load_approved


def load_concept(path: Path):
    try:
        import yaml
    except ImportError as error:
        raise ValueError("PyYAML is required for --okf") from error

    class UniqueKeyLoader(yaml.SafeLoader):
        def construct_mapping(self, node, deep=False):
            keys = set()
            for key_node, _ in node.value:
                key = self.construct_object(key_node)
                if not isinstance(key, str) or key in keys:
                    raise ValueError("OKF front matter requires unique string keys")
                keys.add(key)
            return super().construct_mapping(node, deep=deep)

    with path.open("rb") as file:
        prefix = file.read(65537)
    if len(prefix) > 65536:
        prefix = prefix[:65536].rpartition(b"\n")[0]
    lines = prefix.split(b"\n")
    if lines[0].removesuffix(b"\r") != b"---":
        raise ValueError("Expected OKF YAML front matter")
    end = next((i for i, line in enumerate(lines[1:], 1) if line.removesuffix(b"\r") == b"---"), None)
    if end is None:
        raise ValueError("OKF front matter must end within 64 KiB")
    try:
        metadata = yaml.load(b"\n".join(lines[1:end]).decode("utf-8"), Loader=UniqueKeyLoader)
    except yaml.YAMLError as error:
        raise ValueError(f"Invalid OKF YAML: {error}") from error
    if not isinstance(metadata, dict) or metadata.get("type") != "credential-definition":
        raise ValueError("Expected an OKF credential-definition concept")
    definition = metadata.get("x-ibis-credential")
    if not isinstance(definition, dict) or definition.keys() != {"version", "approved_mapping"}:
        raise ValueError("Expected version and approved_mapping in x-ibis-credential")
    if type(definition["version"]) is not int or definition["version"] != 1:
        raise ValueError("Unsupported x-ibis-credential version")
    ref = definition["approved_mapping"]
    if not isinstance(ref, str) or not ref or "\\" in ref or ":" in ref:
        raise ValueError("approved_mapping must be a local relative JSON path")
    relative = Path(ref)
    if relative.is_absolute() or ".." in relative.parts or relative.suffix != ".json":
        raise ValueError("approved_mapping must be a local relative JSON path")
    directory = path.resolve().parent
    target = (directory / relative).resolve(strict=True)
    if not target.is_relative_to(directory):
        raise ValueError("approved_mapping must stay within the concept directory")
    return load_approved(target)
