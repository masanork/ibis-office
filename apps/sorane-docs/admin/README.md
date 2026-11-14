# Sorane Docs sample administration (local prototype)

This Worker is a separately deployable OIDC relying party and sample service.
It uses Mikaki's confidential-client Authorization Code flow with PKCE. A valid
Mikaki login is not an administrator grant: `sample_admin` must also contain
that issuer and this RP's pairwise subject. A logged-in account can redeem a
short-lived, single-use invitation created by the operator; authentication alone
does not grant access.

The production configuration references dedicated Sorane Docs resources.
The local Wrangler configuration uses only local D1 and R2. Do not substitute production database IDs, credentials or
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

The operator can run `node scripts/prepare-invitation.mjs` from this directory
to prepare private temporary files containing a 32-byte random token and its
SQL digest, with a 15-minute expiry. This does not apply the SQL or transmit
the token. Only the digest and expiry belong in `sample_admin_invite`; convey
the token securely to the intended person and remove the temporary files. A signed-in person redeems it
via a CSRF-protected POST. Both enrollment and deletion of the token run in one
D1 transaction. There is no public first-user grant. Determine production token
delivery, auditing, operator enrollment and recovery before activation. Changes to the production OP client
inventory, D1, R2, routes, or Worker secrets are separate deployment steps.

## Scope and remaining work

Uploads are limited to 2 MiB and accepted only for DOCX/XLSX/PPTX containers
with the expected ZIP directory entries. This is a structural check, not a full
OOXML validator or malware scan. Samples are intended for publication: upload
makes their list and bytes publicly readable. A failed D1 result can leave an
unreferenced R2 object for later reconciliation, so cleanup needs its own
safe, separately designed process.

The home renders the dynamic sample list through the service-binding router in
`worker/sorane-gateway.ts`. Production activation requires service deployment
and credentials, separate production resources and credentials,
operator enrollment, observability, privacy and content review, and an actual
end-to-end Mikaki login, authorization, upload, readback and logout check. The
current tests include a mock issuer and do not establish real Passkey behavior.

## Private user files (prototype)

Authenticated Mikaki users can keep their own DOCX/XLSX/PPTX objects in a
separate R2 bucket (`FILES`). The list and editor round-trip live at `/files`
and `/api/files`. Production still needs the `sorane-docs-files` bucket and
the `0005_user_files.sql` D1 migration; this repository change does not
create either. See `docs/sorane-user-files.md` for operator steps, limits and
the local procedure.
