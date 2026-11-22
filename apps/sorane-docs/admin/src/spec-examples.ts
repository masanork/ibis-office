// SPDX-License-Identifier: AGPL-3.0-only
import { validate, type Schema } from '@cfworker/json-schema';
import { isMap, isNode, isScalar, isSeq, parseAllDocuments, type YAMLMap } from 'yaml';
import { escape } from '../vendor/mikaki-oidc';
import { openApiOperations, openApiVersion, pointerLine, pointerTarget } from './spec-navigation';
import { schemaReference } from './spec-reference';

type Json = null | boolean | string | number | Json[] | { [key: string]: Json };
class UnsupportedExample extends Error {}

function asJson(node: unknown, budget = { remaining: 512 }, depth = 0): Json {
  if (depth > 16 || budget.remaining-- <= 0 || (isNode(node) && node.tag)) throw new UnsupportedExample();
  if (node === null) return null;
  if (isScalar(node)) {
    const value = node.value;
    if (value === null || typeof value === 'string' || typeof value === 'boolean' ||
      (typeof value === 'number' && Number.isFinite(value) && (!Number.isInteger(value) || Number.isSafeInteger(value)))) return value;
  }
  if (isSeq(node)) return node.items.map(item => asJson(item, budget, depth + 1));
  if (isMap(node)) {
    const items: { [key: string]: Json } = Object.create(null);
    for (const { key, value } of node.items) {
      if (!isScalar(key) || typeof key.value !== 'string' || Object.hasOwn(items, key.value)) throw new UnsupportedExample();
      items[key.value] = asJson(value, budget, depth + 1);
    }
    return items;
  }
  throw new UnsupportedExample();
}

function isObject(value: Json): value is { [key: string]: Json } {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

const TYPES = new Set(['null', 'boolean', 'object', 'array', 'number', 'string', 'integer']);
const COUNT_KEYWORDS = new Set(['minLength', 'maxLength', 'minItems', 'maxItems', 'minProperties', 'maxProperties']);
const NUMBER_KEYWORDS = new Set(['minimum', 'maximum', 'exclusiveMinimum', 'exclusiveMaximum']);
const ANNOTATIONS = new Set(['title', 'description', 'default', 'examples', 'example', 'deprecated', '$comment']);

function isSupportedSchema(value: Json): value is boolean | Schema {
  if (typeof value === 'boolean') return true;
  if (!isObject(value)) return false;
  return Object.entries(value).every(([key, field]) => {
    if (ANNOTATIONS.has(key) || key.startsWith('x-')) return true;
    if (key === 'type') {
      const types = Array.isArray(field) ? field : [field];
      return types.length > 0 && types.every(type => typeof type === 'string' && TYPES.has(type));
    }
    if (key === 'required') return Array.isArray(field) && field.every(name => typeof name === 'string');
    if (key === 'properties') return isObject(field) && Object.values(field).every(isSupportedSchema);
    if (key === 'items' || key === 'additionalProperties') return isSupportedSchema(field);
    if (key === 'const') return true;
    if (key === 'enum') return Array.isArray(field) && field.length > 0;
    if (COUNT_KEYWORDS.has(key)) return typeof field === 'number' && Number.isSafeInteger(field) && field >= 0;
    if (NUMBER_KEYWORDS.has(key)) return typeof field === 'number';
    return false;
  });
}

// Materialize only supported schema positions. A $ref inside an example, enum
// value, or extension is data and is never followed.
interface SchemaDocument {
  path: string;
  root: unknown;
  entry: boolean;
}

const DRAFT_2020_12 = 'https://json-schema.org/draft/2020-12/schema';

async function expandSchema(value: Json, doc: SchemaDocument, load: (path: string) => Promise<SchemaDocument>, budget: { remaining: number }, depth = 0): Promise<Json> {
  if (depth > 16 || budget.remaining-- <= 0) throw new UnsupportedExample();
  if (!isObject(value)) return value;
  if (Object.hasOwn(value, '$ref')) {
    if (isMap(doc.root) && (doc.root.has('$id') || doc.root.has('$self'))) throw new UnsupportedExample();
    if (Object.keys(value).some(key => key !== '$ref' && !ANNOTATIONS.has(key) && !key.startsWith('x-'))) {
      throw new UnsupportedExample();
    }
    const ref = typeof value.$ref === 'string' ? schemaReference(doc.path, value.$ref) : null;
    if (!ref) throw new UnsupportedExample();
    const targetDoc = ref.path === doc.path ? doc : await load(ref.path);
    if (targetDoc.entry && !/^\/components\/schemas\/[^/]+$/.test(ref.pointer)) throw new UnsupportedExample();
    if (isMap(targetDoc.root) && (targetDoc.root.has('$id') || targetDoc.root.has('$self'))) throw new UnsupportedExample();
    const target = pointerTarget(targetDoc.root, ref.pointer, true);
    if (!target) throw new UnsupportedExample();
    const schema = asJson(target.node, budget, depth + 1);
    if (!targetDoc.entry && !ref.pointer && isObject(schema) && schema.$schema === DRAFT_2020_12) delete schema.$schema;
    return expandSchema(schema, targetDoc, load, budget, depth + 1);
  }
  // Unsupported keywords still reach the schema checker and cause a skip.
  if (Object.hasOwn(value, '$id') || Object.hasOwn(value, '$schema')) throw new UnsupportedExample();
  const properties = value.properties;
  if (isObject(properties)) {
    for (const [key, property] of Object.entries(properties)) {
      properties[key] = await expandSchema(property, doc, load, budget, depth + 1);
    }
  }
  for (const key of ['items', 'additionalProperties']) {
    if (Object.hasOwn(value, key)) value[key] = await expandSchema(value[key], doc, load, budget, depth + 1);
  }
  return value;
}

function* mediaExamples(contentOwner: unknown) {
  if (!isMap(contentOwner)) return;
  const content = contentOwner.get('content', true);
  if (!isMap(content)) return;
  for (const { key, value } of content.items) {
    if (!isScalar(key) || typeof key.value !== 'string' ||
      !/^application\/(?:json|[\w.+-]+\+json)$/i.test(key.value) || !isMap(value)) continue;
    const schema = value.get('schema', true);
    if (value.has('example')) yield { schema, node: value.get('example', true), anchor: value };
    const examples = value.get('examples', true);
    if (!isMap(examples)) continue;
    for (const { key, value } of examples.items) {
      if (!isMap(value) || value.has('$ref') || !value.has('value')) continue;
      yield { schema, node: value.get('value', true), anchor: key };
    }
  }
}

function* operationExamples(operation: YAMLMap) {
  yield* mediaExamples(operation.get('requestBody', true));
  const responses = operation.get('responses', true);
  if (!isMap(responses)) return;
  for (const { value } of responses.items) yield* mediaExamples(value);
}

export async function renderOpenApiExamples(root: unknown, location: (offset: number) => number, source: string, loadSchema: (path: string) => Promise<string | null>): Promise<string> {
  if (!isMap(root)) return '';
  const version = openApiVersion(root.get('openapi'));
  const paths = root.get('paths', true);
  if (!version || version.startsWith('3.0.') || !isMap(paths)) return '';
  const dialect = root.get('jsonSchemaDialect');
  const knownDialect = !root.has('jsonSchemaDialect') ||
    dialect === DRAFT_2020_12 ||
    dialect === `https://spec.openapis.org/oas/${version.slice(0, 3)}/dialect/base`;

  const entry: SchemaDocument = { path: source, root, entry: true };
  const files = new Map<string, Promise<SchemaDocument>>();
  async function load(path: string): Promise<SchemaDocument> {
    if (path === source) return entry;
    const cached = files.get(path);
    if (cached) return cached;
    if (files.size >= 4) throw new UnsupportedExample();
    const pending = readSchema(path);
    files.set(path, pending);
    return pending;
  }

  async function readSchema(path: string): Promise<SchemaDocument> {
    const raw = await loadSchema(path);
    if (raw === null || raw.length > 128 * 1024) throw new UnsupportedExample();
    if (/\.json$/i.test(path)) JSON.parse(raw);
    const documents = parseAllDocuments(raw, { prettyErrors: false, uniqueKeys: false });
    if (documents.length !== 1 || documents[0].errors.length || !documents[0].contents) throw new UnsupportedExample();
    const root = documents[0].contents;
    const dialect: unknown = isMap(root) ? root.get('$schema', true) : undefined;
    if (isMap(root) && (root.has('openapi') ||
      (root.has('$schema') && (!isScalar(dialect) || dialect.value !== DRAFT_2020_12)))) {
      throw new UnsupportedExample();
    }
    return { path, root, entry: false };
  }

  let examined = 0;
  let validated = 0;
  let truncated = false;
  const issues: string[] = [];
  operations: for (const { path, method, value } of openApiOperations(paths, version)) {
    if (!isMap(value)) continue;
    for (const { schema, node, anchor } of operationExamples(value)) {
      if (examined === 20) { truncated = true; break operations; }
      examined++;
      try {
        if (!knownDialect) continue;
        const budget = { remaining: 1024 };
        const parsed = await expandSchema(asJson(schema, budget), entry, load, budget);
        if (!isSupportedSchema(parsed)) continue;
        const instance = asJson(node);
        const result = validate(instance, parsed, '2020-12', Object.create(null));
        validated++;
        if (result.valid) continue;
        const error = result.errors.at(-1);
        const reference = decodeURIComponent(error?.instanceLocation.slice(1) ?? '');
        const line = pointerLine(node, reference, location) ??
          location((isNode(anchor) ? anchor.range?.[0] : undefined) ?? 0);
        const issue = (error?.error ?? 'スキーマに適合しません。').slice(0, 220);
        issues.push(`<li><a href="#L${line}">${line} 行目</a> · <code>${escape(method)} ${escape(path)}</code> · ${escape(issue)}</li>`);
      } catch {
        // A non-JSON value or unsupported schema must never hide the source pane.
      }
    }
  }
  if (!examined) return '';
  return `<aside class="source-structure" aria-label="OpenAPI JSON 例チェック"><h2>OpenAPI JSON 例チェック</h2>
    <p>${validated} 件を検証し、${issues.length} 件に問題が見つかりました。</p>
    ${issues.length ? `<ul>${issues.join('')}</ul>` : ''}
    ${examined > validated ? `<p class="muted">${examined - validated} 件は対応するスキーマ参照やキーワードなどの条件を満たさず、検証対象外です。</p>` : ''}
    ${truncated ? '<p class="muted">先頭20件まで確認しました。</p>' : ''}</aside>`;
}
