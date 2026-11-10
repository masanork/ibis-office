# Experimental JTD preview

The browser can open `.jtd` files locally as a read-only, text-only preview.
It reads the observed `DocumentText` stream into ibisOffice's document model;
formatting, layout, tables, pictures and some text controls are not reproduced.
Unsupported or encrypted variants may not open. `.jtt` and `.jttc` are not supported.
The source bytes remain in the browser; the optional embedding download action
returns the original bytes, not a re-created JTD. Saving or editing as JTD is
not supported.

The independent importer uses format observations in OpenJTD RFC 0003 and
0009, revision `cfb44c9ac53126e00808c49709fb4dade8a4daae`:
https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc
These draft observations do not establish every record's semantics. Unknown
constructs are omitted from the preview; it is not a faithful document rendering.
Synthetic fixtures are used in tests. Local samples must not be redistributed
without permission.
