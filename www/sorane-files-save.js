// SPDX-License-Identifier: AGPL-3.0-only
// When the editor was opened from a private file, the embed download event
// writes the current document back with If-Match. File > Save in the editor
// itself is not yet available in the browser version.
const open = new URLSearchParams(location.search).get('open');
const file = open && /^\/api\/files\/([0-9a-f-]{36})\/([A-Za-z0-9._-]{1,80}\.(?:docx|xlsx|pptx))$/i.exec(open);
if (file) {
  const path = `/api/files/${file[1]}/${file[2]}`;
  let csrf = '';
  let etag = '';
  const banner = document.createElement('div');
  banner.setAttribute('role', 'status');
  banner.style.cssText = 'display:none;position:fixed;left:0;right:0;top:0;z-index:1000;padding:.7rem 1rem;font:14px system-ui,sans-serif;text-align:center';
  const attach = () => { if (!banner.isConnected && document.body) document.body.append(banner); };
  if (document.body) attach();
  else document.addEventListener('DOMContentLoaded', attach);
  const show = (text, error) => {
    banner.style.display = 'block';
    banner.style.background = error ? '#fde8e8' : '#e8f4ea';
    banner.style.color = error ? '#9b1c1c' : '#14532d';
    banner.textContent = text;
  };
  const headers = { Accept: 'application/json' };
  const session = await fetch('/api/files', { headers, credentials: 'same-origin' });
  if (session.status === 401) {
    show('保存するにはログインしてください。保存したファイルの画面から入り直してください。', true);
  } else if (session.ok) {
    const body = await session.json();
    if (body && typeof body.csrf === 'string') csrf = body.csrf;
  }
  try {
    const head = await fetch(path, { method: 'HEAD', credentials: 'same-origin' });
    if (head.ok) etag = head.headers.get('ETag') ?? '';
  } catch { /* The editor may still open the file; a later save will surface the error. */ }
  window.addEventListener('ibisoffice:download', async (event) => {
    const bytes = event.detail?.bytes;
    if (!bytes) return;
    if (!csrf) {
      show('保存用の情報が取れませんでした。保存したファイルの画面から開き直してください。', true);
      return;
    }
    if (!etag) {
      show('ファイルの版情報が取れませんでした。開き直してから書き戻してください。', true);
      return;
    }
    show('クラウドへ書き戻しています…');
    try {
      const response = await fetch(path, {
        method: 'PUT',
        credentials: 'same-origin',
        headers: { 'x-csrf-token': csrf, 'If-Match': etag },
        body: bytes,
      });
      if (response.ok) {
        const saved = await response.json().catch(() => ({}));
        if (typeof saved.etag === 'string' && saved.etag) etag = saved.etag;
        show('クラウドへ書き戻しました。');
        return;
      }
      if (response.status === 412 || response.status === 409) {
        show('ほかで更新されているため書き戻せませんでした。保存したファイルの一覧から開き直してください。', true);
        return;
      }
      if (response.status === 401) {
        show('セッションが切れたため書き戻せませんでした。保存したファイルの画面から入り直してください。', true);
        return;
      }
      show('書き戻せませんでした。', true);
    } catch {
      show('通信できず、書き戻せませんでした。', true);
    }
  });
}
