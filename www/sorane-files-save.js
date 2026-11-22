// SPDX-License-Identifier: AGPL-3.0-only
// When the editor was opened from a private file or a repo blob, the embed
// download event writes the current document back with If-Match. For repo blobs,
// a commit modal previews the Markdown/text diff, lets the user pick a branch
// and commit message, and offers a branch-off escape hatch on 412 conflicts.
const open = new URLSearchParams(location.search).get('open');
const file = open && /^\/api\/files\/([0-9a-f-]{36})\/([A-Za-z0-9._-]{1,80}\.(?:docx|xlsx|pptx))$/i.exec(open);
let repoInfo = null;
if (open) {
  try {
    const parsedOpen = new URL(open, location.origin);
    const m = /^\/api\/repos\/([0-9a-f-]{36})\/blob\/(.+)$/i.exec(parsedOpen.pathname);
    if (m) {
      repoInfo = {
        repoId: m[1],
        rawBlobPath: m[2],
        blobPath: decodeURIComponent(m[2]),
        ref: parsedOpen.searchParams.get('ref')?.trim() || '',
        url: parsedOpen.pathname + parsedOpen.search,
      };
    }
  } catch { /* ignore malformed open param */ }
}

if (file || repoInfo) {
  const path = file ? `/api/files/${file[1]}/${file[2]}` : repoInfo.url;
  const csrfEndpoint = file ? '/api/files' : '/api/repos';
  const backLabel = file ? '保存したファイルの画面' : 'リポジトリの画面';
  const isTextRepoFile = Boolean(
    repoInfo && /\.(md|markdown|txt|json|xml|yaml|yml|csv|tsv|html|htm|css|js|ts|toml)$/i.test(repoInfo.blobPath),
  );
  let csrf = '';
  let etag = '';
  let originalText = '';
  let originalSize = 0;
  let branches = [];
  let defaultBranch = 'main';
  let currentBranch = repoInfo?.ref || '';

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
  const session = await fetch(csrfEndpoint, { headers, credentials: 'same-origin' });
  if (session.status === 401) {
    show(`保存するにはログインしてください。${backLabel}から入り直してください。`, true);
  } else if (session.ok) {
    const body = await session.json();
    if (body && typeof body.csrf === 'string') csrf = body.csrf;
  }

  try {
    if (repoInfo) {
      const [blobRes, branchRes] = await Promise.all([
        fetch(path, { method: 'GET', credentials: 'same-origin' }),
        fetch(`/api/repos/${repoInfo.repoId}/branches`, { headers, credentials: 'same-origin' }),
      ]);
      if (blobRes.ok) {
        etag = blobRes.headers.get('ETag') ?? '';
        const buf = new Uint8Array(await blobRes.arrayBuffer());
        originalSize = buf.length;
        if (isTextRepoFile) originalText = new TextDecoder().decode(buf);
      }
      if (branchRes.ok) {
        const bData = await branchRes.json().catch(() => ({}));
        if (typeof bData.defaultBranch === 'string' && bData.defaultBranch) {
          defaultBranch = bData.defaultBranch;
        }
        if (Array.isArray(bData.branches)) branches = bData.branches;
      }
      if (!currentBranch) currentBranch = defaultBranch;
    } else {
      const head = await fetch(path, { method: 'HEAD', credentials: 'same-origin' });
      if (head.ok) etag = head.headers.get('ETag') ?? '';
    }
  } catch { /* The editor may still open the file; a later save will surface the error. */ }

  function splitLines(text) {
    if (!text) return [];
    const lines = text.replace(/\r\n/g, '\n').replace(/\r/g, '\n').split('\n');
    if (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines;
  }

  function computeSimpleDiff(oldStr, newStr) {
    const oldLines = splitLines(oldStr);
    const newLines = splitLines(newStr);
    let prefix = 0;
    const minLen = Math.min(oldLines.length, newLines.length);
    while (prefix < minLen && oldLines[prefix] === newLines[prefix]) prefix++;
    let suffix = 0;
    while (
      suffix < minLen - prefix &&
      oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
    ) {
      suffix++;
    }
    const midOld = oldLines.slice(prefix, oldLines.length - suffix);
    const midNew = newLines.slice(prefix, newLines.length - suffix);
    if (!midOld.length && !midNew.length) return { ops: [], additions: 0, deletions: 0 };

    let midOps;
    if (midOld.length * midNew.length > 200000) {
      midOps = [
        ...midOld.map((text, idx) => ({ type: 'del', oldLine: prefix + idx + 1, newLine: null, text })),
        ...midNew.map((text, idx) => ({ type: 'add', oldLine: null, newLine: prefix + idx + 1, text })),
      ];
    } else {
      const n = midOld.length;
      const m = midNew.length;
      const w = m + 1;
      const dp = new Int32Array((n + 1) * w);
      for (let i = n - 1; i >= 0; i--) {
        for (let j = m - 1; j >= 0; j--) {
          dp[i * w + j] = midOld[i] === midNew[j]
            ? dp[(i + 1) * w + j + 1] + 1
            : Math.max(dp[(i + 1) * w + j], dp[i * w + j + 1]);
        }
      }
      midOps = [];
      let i = 0;
      let j = 0;
      while (i < n && j < m) {
        if (midOld[i] === midNew[j]) {
          midOps.push({ type: 'context', oldLine: prefix + i + 1, newLine: prefix + j + 1, text: midOld[i] });
          i++;
          j++;
        } else if (dp[(i + 1) * w + j] >= dp[i * w + j + 1]) {
          midOps.push({ type: 'del', oldLine: prefix + i + 1, newLine: null, text: midOld[i] });
          i++;
        } else {
          midOps.push({ type: 'add', oldLine: null, newLine: prefix + j + 1, text: midNew[j] });
          j++;
        }
      }
      while (i < n) {
        midOps.push({ type: 'del', oldLine: prefix + i + 1, newLine: null, text: midOld[i] });
        i++;
      }
      while (j < m) {
        midOps.push({ type: 'add', oldLine: null, newLine: prefix + j + 1, text: midNew[j] });
        j++;
      }
    }

    const ops = [];
    for (let idx = Math.max(0, prefix - 3); idx < prefix; idx++) {
      ops.push({ type: 'context', oldLine: idx + 1, newLine: idx + 1, text: oldLines[idx] });
    }
    ops.push(...midOps);
    for (let k = 0; k < Math.min(suffix, 3); k++) {
      const oi = oldLines.length - suffix + k;
      const ni = newLines.length - suffix + k;
      ops.push({ type: 'context', oldLine: oi + 1, newLine: ni + 1, text: oldLines[oi] });
    }
    let additions = 0;
    let deletions = 0;
    for (const op of ops) {
      if (op.type === 'add') additions++;
      else if (op.type === 'del') deletions++;
    }
    return { ops, additions, deletions };
  }

  function renderDiffTable(container, diff) {
    container.innerHTML = '';
    if (!diff.ops.length) {
      const p = document.createElement('p');
      p.style.cssText = 'margin:0;color:#656d76;font-size:13px';
      p.textContent = '変更はありません。';
      container.append(p);
      return;
    }
    const stat = document.createElement('div');
    stat.style.cssText = 'font-size:12px;color:#656d76;margin-bottom:6px';
    stat.textContent = `+${diff.additions} 行追加 / -${diff.deletions} 行削除`;
    const table = document.createElement('table');
    table.style.cssText = 'width:100%;border-collapse:collapse;font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace';
    for (const op of diff.ops) {
      const tr = document.createElement('tr');
      if (op.type === 'add') tr.style.background = '#e6ffec';
      else if (op.type === 'del') tr.style.background = '#ffebe9';
      const td1 = document.createElement('td');
      td1.style.cssText = 'width:34px;text-align:right;padding:1px 6px;color:#656d76;border-right:1px solid #d0d7de;user-select:none';
      td1.textContent = op.oldLine ?? '';
      const td2 = document.createElement('td');
      td2.style.cssText = 'width:34px;text-align:right;padding:1px 6px;color:#656d76;border-right:1px solid #d0d7de;user-select:none';
      td2.textContent = op.newLine ?? '';
      const td3 = document.createElement('td');
      td3.style.cssText = 'padding:1px 8px;white-space:pre-wrap;word-break:break-word';
      td3.textContent = (op.type === 'add' ? '+' : op.type === 'del' ? '-' : ' ') + op.text;
      tr.append(td1, td2, td3);
      table.append(tr);
    }
    container.append(stat, table);
  }

  async function commitRepoBytes(bytes, { targetBranch, baseRef, message, useIfMatch }) {
    const reqHeaders = {
      'x-csrf-token': csrf,
      'x-commit-message': encodeURIComponent(message),
      'x-target-branch': targetBranch,
      'x-base-ref': baseRef,
    };
    if (useIfMatch && etag) reqHeaders['If-Match'] = etag;
    const baseEndpoint = `/api/repos/${repoInfo.repoId}/blob/${repoInfo.rawBlobPath}`;
    return await fetch(baseEndpoint, {
      method: 'PUT',
      credentials: 'same-origin',
      headers: reqHeaders,
      body: bytes,
    });
  }

  function openRepoCommitModal(bytes) {
    const newText = isTextRepoFile ? new TextDecoder().decode(bytes) : '';
    const overlay = document.createElement('div');
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.style.cssText = 'position:fixed;inset:0;z-index:1100;background:rgba(15,23,42,.5);display:flex;align-items:center;justify-content:center;padding:16px;font:14px/1.5 system-ui,-apple-system,sans-serif;color:#1f2328';

    const card = document.createElement('div');
    card.style.cssText = 'background:#fff;border-radius:10px;max-width:720px;width:100%;max-height:88vh;display:flex;flex-direction:column;box-shadow:0 20px 50px rgba(0,0,0,.25);overflow:hidden';

    const hd = document.createElement('div');
    hd.style.cssText = 'padding:12px 16px;background:#f6f8fa;border-bottom:1px solid #d0d7de;display:flex;justify-content:space-between;align-items:center';
    const title = document.createElement('strong');
    title.textContent = `Git コミット · ${repoInfo.blobPath}`;
    const closeBtn = document.createElement('button');
    closeBtn.type = 'button';
    closeBtn.textContent = '閉じる';
    closeBtn.style.cssText = 'padding:4px 10px;border:1px solid #d0d7de;border-radius:6px;background:#fff;cursor:pointer';
    closeBtn.addEventListener('click', () => overlay.remove());
    hd.append(title, closeBtn);

    const body = document.createElement('div');
    body.style.cssText = 'padding:16px;overflow:auto;display:flex;flex-direction:column;gap:12px';

    const diffWrap = document.createElement('div');
    diffWrap.style.cssText = 'border:1px solid #d0d7de;border-radius:6px;padding:10px;max-height:280px;overflow:auto;background:#fff';
    if (isTextRepoFile) {
      renderDiffTable(diffWrap, computeSimpleDiff(originalText, newText));
    } else {
      diffWrap.textContent = `バイナリファイルサイズ: ${originalSize} bytes → ${bytes.length} bytes`;
    }

    const msgLabel = document.createElement('label');
    msgLabel.style.cssText = 'display:block;font-weight:600';
    msgLabel.textContent = 'コミットメッセージ';
    const msgInput = document.createElement('input');
    msgInput.type = 'text';
    msgInput.value = `Update ${repoInfo.blobPath}`;
    msgInput.style.cssText = 'margin-top:4px;width:100%;box-sizing:border-box;padding:7px 10px;border:1px solid #d0d7de;border-radius:6px;font:inherit';
    msgLabel.append(msgInput);

    const branchRow = document.createElement('div');
    branchRow.style.cssText = 'display:grid;grid-template-columns:1fr 1fr;gap:10px';
    const selLabel = document.createElement('label');
    selLabel.style.cssText = 'display:block;font-weight:600';
    selLabel.textContent = '保存先ブランチ';
    const branchSel = document.createElement('select');
    branchSel.style.cssText = 'margin-top:4px;width:100%;padding:7px 10px;border:1px solid #d0d7de;border-radius:6px;font:inherit;background:#fff';
    const seen = new Set();
    for (const b of [currentBranch, ...branches.map((x) => x.name)].filter(Boolean)) {
      if (seen.has(b)) continue;
      seen.add(b);
      const opt = document.createElement('option');
      opt.value = b;
      opt.textContent = b === defaultBranch ? `${b} (default)` : b;
      if (b === currentBranch) opt.selected = true;
      branchSel.append(opt);
    }
    const newOpt = document.createElement('option');
    newOpt.value = '__new__';
    newOpt.textContent = '+ 新しいブランチを作成…';
    branchSel.append(newOpt);
    selLabel.append(branchSel);

    const newBranchLabel = document.createElement('label');
    newBranchLabel.style.cssText = 'display:none;font-weight:600';
    newBranchLabel.textContent = '新規ブランチ名';
    const newBranchInput = document.createElement('input');
    newBranchInput.type = 'text';
    newBranchInput.placeholder = 'draft/edit-1';
    newBranchInput.style.cssText = 'margin-top:4px;width:100%;box-sizing:border-box;padding:7px 10px;border:1px solid #d0d7de;border-radius:6px;font:inherit';
    newBranchLabel.append(newBranchInput);
    branchSel.addEventListener('change', () => {
      newBranchLabel.style.display = branchSel.value === '__new__' ? 'block' : 'none';
    });
    branchRow.append(selLabel, newBranchLabel);

    const statusMsg = document.createElement('div');
    statusMsg.style.cssText = 'font-size:13px;min-height:18px';

    const conflictBox = document.createElement('div');
    conflictBox.style.cssText = 'display:none;gap:8px;flex-wrap:wrap;align-items:center;background:#ffebe9;border:1px solid #ff8182;padding:10px;border-radius:6px';

    const ft = document.createElement('div');
    ft.style.cssText = 'padding:12px 16px;background:#f6f8fa;border-top:1px solid #d0d7de;display:flex;justify-content:flex-end;gap:8px';
    const cancelBtn = document.createElement('button');
    cancelBtn.type = 'button';
    cancelBtn.textContent = 'キャンセル';
    cancelBtn.style.cssText = 'padding:7px 14px;border:1px solid #d0d7de;border-radius:6px;background:#fff;cursor:pointer';
    cancelBtn.addEventListener('click', () => overlay.remove());
    const commitBtn = document.createElement('button');
    commitBtn.type = 'button';
    commitBtn.textContent = 'Commit & Push';
    commitBtn.style.cssText = 'padding:7px 14px;border:1px solid #1a7f37;border-radius:6px;background:#1a7f37;color:#fff;font-weight:600;cursor:pointer';

    async function runCommit(targetBranch, useIfMatch) {
      const msg = msgInput.value.trim() || `Update ${repoInfo.blobPath}`;
      commitBtn.disabled = true;
      statusMsg.style.color = '#656d76';
      statusMsg.textContent = `${targetBranch} へコミットしています…`;
      conflictBox.style.display = 'none';
      try {
        const res = await commitRepoBytes(bytes, {
          targetBranch,
          baseRef: currentBranch || defaultBranch,
          message: msg,
          useIfMatch,
        });
        if (res.ok) {
          const saved = await res.json().catch(() => ({}));
          if (typeof saved.etag === 'string' && saved.etag) etag = saved.etag;
          currentBranch = targetBranch;
          if (isTextRepoFile) originalText = newText;
          originalSize = bytes.length;
          overlay.remove();
          show(`${targetBranch} にコミットしました (${String(saved.commit || '').slice(0, 7)})。`);
          return;
        }
        if (res.status === 412 || res.status === 409) {
          statusMsg.style.color = '#cf222e';
          statusMsg.textContent = 'ほかで先に更新されたため競合しました（412 Conflict）。';
          conflictBox.innerHTML = '';
          conflictBox.style.display = 'flex';
          const branchOffBtn = document.createElement('button');
          branchOffBtn.type = 'button';
          branchOffBtn.textContent = '別ブランチを作成して保存';
          branchOffBtn.style.cssText = 'padding:6px 12px;border:1px solid #0969da;border-radius:6px;background:#0969da;color:#fff;cursor:pointer';
          branchOffBtn.addEventListener('click', async () => {
            const suggested = `draft-${new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')}`;
            branchSel.value = '__new__';
            newBranchLabel.style.display = 'block';
            newBranchInput.value = suggested;
            await runCommit(suggested, false);
          });
          conflictBox.append(branchOffBtn);
        } else if (res.status === 401) {
          statusMsg.style.color = '#cf222e';
          statusMsg.textContent = 'セッションが切れました。リポジトリ画面からログインし直してください。';
        } else {
          const err = await res.json().catch(() => ({}));
          statusMsg.style.color = '#cf222e';
          statusMsg.textContent = `コミットに失敗しました: ${err.error || res.status}`;
        }
      } catch {
        statusMsg.style.color = '#cf222e';
        statusMsg.textContent = '通信エラーが発生しました。';
      } finally {
        commitBtn.disabled = false;
      }
    }

    commitBtn.addEventListener('click', () => {
      const isNew = branchSel.value === '__new__';
      const target = isNew ? newBranchInput.value.trim() : branchSel.value;
      if (!target) {
        statusMsg.style.color = '#cf222e';
        statusMsg.textContent = '新しいブランチ名を入力してください。';
        return;
      }
      runCommit(target, !isNew);
    });

    ft.append(cancelBtn, commitBtn);
    body.append(diffWrap, branchRow, msgLabel, statusMsg, conflictBox);
    card.append(hd, body, ft);
    overlay.append(card);
    document.body.append(overlay);
  }

  window.addEventListener('ibisoffice:download', async (event) => {
    const bytes = event.detail?.bytes;
    if (!bytes) return;
    if (!csrf) {
      show(`保存用の情報が取れませんでした。${backLabel}から開き直してください。`, true);
      return;
    }
    if (repoInfo) {
      openRepoCommitModal(bytes);
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
