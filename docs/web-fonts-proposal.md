# Optional browser font delivery

The web build can use a self-hosted font manifest by setting `IBIS_WEB_FONTS_CONFIG` to a JSON configuration file before running `scripts/build-web.sh`. `scripts/web-fonts.example.json` shows the configuration format. Font files and their distribution rights must be supplied by the deployer. The normal build does not select a font source or subset service.

The build can subset configured bootstrap fonts, include glyphs used by UI resources, preserve attribution files, and generate a content-addressed manifest. The loader verifies font hashes. Optional dynamic delivery requests missing characters from a configured endpoint, installs the returned subset and refreshes document layout. The document repertoire includes content outside the current viewport. The example `worker/font-service.ts` separates storage, caching, rate limiting and the subset engine behind injected interfaces; it is not a deployed service.

For single-face delivery, set `dynamic` to an object with `endpoint` and `font`. The optional `faces` strategy currently contains selection rules for the `ipamjm`, `NotoSansCJKJP` and `acgjm` IDs; adapting it to arbitrary face sets requires additional work. Requests send requested characters to the configured service, so deployments should choose and disclose that service with document confidentiality in mind. Different fonts have different redistribution licenses.

The GPUI WGPU dependency points to a public branch containing the font-registration changes required for this proposal. Those changes would need to be accepted upstream or otherwise integrated before adoption. The sample service and tests use injected local dependencies; production deployment, operating limits, and physical-device behavior require separate validation.
