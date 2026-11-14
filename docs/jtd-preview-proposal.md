# Experimental Ichitaro preview

The desktop and web editor can open supported `.jtd`, `.jtt` and observed
LH5-compressed `.jttc` as a read-only preview. The source remains protected.
The user can export recognized text as DOCX or HTML, or convert the preview to
an editable DOCX and save it separately. This conversion is lossy and does
not implement writing or round-tripping the original Ichitaro format.

The importer recognizes selected text, paragraph and character formatting,
vertical writing, ruby and some table/border records. Other records and
geometry remain incomplete. Detected omitted sheets and embedded objects are
reported. The optional image gallery shows observed WMF candidates without
asserting their original placement, page, ordering or size. The saved
thumbnail is a separate snapshot that can be stale and is not evidence of
page fidelity. Unsupported or encrypted variants may not open.

The block decoder follows published observations in
[OpenJTD RFC 0003 and 0009](https://github.com/OpenJTD/rjtd/tree/cfb44c9ac53126e00808c49709fb4dade8a4daae/openjtd-spec/rfc)
and independent observations described in the fork's
[format notes](https://github.com/masanork/ibis-office/blob/b62f5b08897bb7f0cf470762d013c6493700188e/docs/jtd-format-notes.md).
No other project's parser implementation was copied. The included JTD/JTTC
fixtures are synthetic; local authorized samples and matching PDFs are not
included. Passing tests does not establish compatibility with all Ichitaro
versions or rendering fidelity.

## Browser smoke test

After `scripts/build-web.sh`, serve the repository root, then run the optional
Chrome test (Playwright is needed only for this test):

```sh
npm install --no-save --package-lock=false playwright-core@1.64.1
python3 -m http.server 8798 --bind 127.0.0.1
# in another terminal
node scripts/test-jtd-browser.mjs
```

`IBIS_JTD_TEST_ROOT` and `IBIS_CHROME_PATH` override the default root URL and
Chrome binary. The test uses `unzip` to inspect the generated DOCX. It checks
the synthetic JTD via an initial URL and a file picker, byte-identical source
download, DOCX and HTML exports, and the absence of browser panics.
