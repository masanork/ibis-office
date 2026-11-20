# C2PA trust anchors

These unmodified certificate bundles come from the Coalition for Content
Provenance and Authenticity's [official conformance repository](
https://github.com/c2pa-org/conformance-public/tree/92de7d8c2510160fa816d2ae90957e52a9ea62d4/trust-list).
The repository is licensed under [CC BY 4.0](
https://creativecommons.org/licenses/by/4.0/).

| Bundle | SHA-256 |
| --- | --- |
| C2PA-TRUST-LIST.pem | `bcc1a4e74ff7a9c9f2f20af16a6a48d4248359244b2bb246199941262d8ea331` |
| C2PA-TSA-TRUST-LIST.pem | `eca8347ee71e31d226e00c85b15aca356116fefe6c26ed8c0ee1f954115b1990` |

Update both bundles from a reviewed upstream commit. Record its permalink
and the two SHA-256 values here, and ship the update with the desktop and
browser builds. Both builds use these bundled files during verification.
