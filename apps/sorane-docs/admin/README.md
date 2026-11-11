# Sorane Docs sample administration (local prototype)

This Worker is a separately deployable OIDC relying party and sample service.
It uses Mikaki's confidential-client Authorization Code flow with PKCE. A valid
Mikaki login is not an administrator grant: `sample_admin` must also contain
that issuer and this RP's pairwise subject. A logged-in account can see its own
subject on `/admin` to request operator enrollment; it does not enroll itself.

This is not configured for production. The local Wrangler configuration uses
only local D1 and R2. Do not substitute production database IDs, credentials or
keys in it. The checked-in OIDC implementation and its MIT licence are pinned
in `vendor/README.md`.

## Local development

```sh
cd apps/sorane-docs/admin
npm ci
npm run check
npm test
npx wrangler d1 migrations apply sorane-admin-local --config wrangler.local.jsonc --local
```

To run an interactive test, start a disposable OIDC provider registered with
client `sorane-local` and callback `http://127.0.0.1:18879/callback`, generate
its own throwaway ES256 client key, and provide the private JWK to Wrangler via
an external env file. Run `npx wrangler dev --local --config wrangler.local.jsonc
--env-file /private/path/to/test.vars --port 18879 --ip 127.0.0.1`. Its issuer is
`http://localhost:18877`. Test credentials and local subjects must never be
reused for production.

The operator explicitly adds an intended subject to `sample_admin`; no public
first-user or invitation endpoint grants access. Determine production enrollment
and recovery before activating the service. Changes to the production OP client
inventory, D1, R2, routes, or Worker secrets are separate deployment steps.

## Scope and remaining work

Uploads are limited to 2 MiB and accepted only for DOCX/XLSX/PPTX containers
with the expected ZIP directory entries. This is a structural check, not a full
OOXML validator or malware scan. Samples are intended for publication: upload
makes their list and bytes publicly readable. A failed D1 result can leave an
unreferenced R2 object for later reconciliation, so cleanup needs its own
safe, separately designed process.

The public home is not yet connected to the dynamic sample list. Production
requires a same-origin route, separate production resources and credentials,
operator enrollment, observability, privacy and content review, and an actual
end-to-end Mikaki login, authorization, upload, readback and logout check. The
current tests include a mock issuer and do not establish real Passkey behavior.
