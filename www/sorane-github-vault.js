// SPDX-License-Identifier: AGPL-3.0-only
// GitHub App login + Private repository creation/selection and start-view integration for soraneDocs Web.

const params = new URLSearchParams(location.search);
if (!params.has('host')) {
  let session = { configured: false, loggedIn: false, login: null, vault: null, installUrl: null };
  let showingRepo = true;
  let repoItems = [];
  let openedRepoFile = null; // { path, name, etag }

  function badgeFor(name) {
    const ext = name.includes('.') ? name.split('.').pop().toUpperCase() : 'FILE';
    if (ext === 'MARKDOWN') return 'MD';
    return ext.slice(0, 6);
  }

  function applyStartViewConfig() {
    const base = window.soraneStartBase;
    if (!base) return;
    let selectAction;
    if (!session.loggedIn) {
      selectAction = { label: 'GitHub でログイン', action: 'github-vault' };
    } else if (!session.vault) {
      selectAction = { label: `GitHub リポジトリを選択 (@${session.login})`, action: 'github-vault' };
    } else {
      selectAction = { label: `GitHub: ${session.vault.owner}/${session.vault.repo} ▾`, action: 'github-vault' };
    }

    const useRepoList = Boolean(session.loggedIn && session.vault && showingRepo);
    const nextConfig = {
      ...base,
      selectAction,
      listTitle: useRepoList
        ? `GitHub: ${session.vault.owner}/${session.vault.repo} (${session.vault.branch})`
        : base.listTitle,
      items: useRepoList ? repoItems : base.items,
    };
    window.ibisOfficeOptions = {
      ...(window.ibisOfficeOptions || {}),
      startView: nextConfig,
    };
    if (typeof window.ibisOfficeSetStartView === 'function') {
      window.ibisOfficeSetStartView(nextConfig);
    }
  }

  async function refreshRepoTree() {
    if (!session.loggedIn || !session.vault) {
      repoItems = [];
      applyStartViewConfig();
      return;
    }
    try {
      const res = await fetch('/api/github/tree', { credentials: 'same-origin' });
      if (!res.ok) {
        repoItems = [];
        applyStartViewConfig();
        return;
      }
      const data = await res.json();
      repoItems = (data.files || []).map((f) => ({
        title: f.name,
        subtitle: f.path,
        name: f.name,
        url: `/api/github/file?path=${encodeURIComponent(f.path)}`,
        badge: badgeFor(f.name),
      }));
    } catch {
      repoItems = [];
    }
    applyStartViewConfig();
  }

  async function loadSession() {
    try {
      const res = await fetch('/api/github/session', { credentials: 'same-origin' });
      if (res.ok) {
        session = await res.json();
      }
    } catch {
      /* Keep unauthenticated fallback when API is unavailable. */
    }
    applyStartViewConfig();
    if (session.loggedIn && session.vault) {
      await refreshRepoTree();
    }
    if (params.get('github') === 'connected' && session.loggedIn) {
      const clean = new URL(location.href);
      clean.searchParams.delete('github');
      history.replaceState(null, '', clean.toString());
      if (!session.vault) openRepoModal();
    }
  }

  function createModalShell(titleText) {
    const existing = document.getElementById('sorane-gh-modal');
    if (existing) existing.remove();
    const overlay = document.createElement('div');
    overlay.id = 'sorane-gh-modal';
    overlay.style.cssText = 'position:fixed;inset:0;z-index:2000;background:rgba(15,23,42,.48);display:flex;align-items:center;justify-content:center;padding:16px;font:14px/1.5 system-ui,-apple-system,sans-serif;color:#0f172a';
    const dialog = document.createElement('div');
    dialog.setAttribute('role', 'dialog');
    dialog.setAttribute('aria-modal', 'true');
    dialog.style.cssText = 'width:min(560px,100%);max-height:86vh;overflow:auto;background:#fff;border-radius:14px;box-shadow:0 20px 50px rgba(15,23,42,.25);padding:20px 22px;display:flex;flex-direction:column;gap:14px';
    const header = document.createElement('div');
    header.style.cssText = 'display:flex;align-items:center;justify-content:space-between;gap:12px';
    const title = document.createElement('h2');
    title.textContent = titleText;
    title.style.cssText = 'margin:0;font-size:17px;font-weight:700';
    const close = document.createElement('button');
    close.type = 'button';
    close.textContent = '✕';
    close.setAttribute('aria-label', '閉じる');
    close.style.cssText = 'border:0;background:transparent;font-size:18px;cursor:pointer;color:#64748b;padding:4px 8px';
    close.onclick = () => overlay.remove();
    overlay.onclick = (e) => { if (e.target === overlay) overlay.remove(); };
    header.append(title, close);
    dialog.append(header);
    overlay.append(dialog);
    document.body.append(overlay);
    return { overlay, dialog };
  }

  async function openRepoModal() {
    if (!session.configured) {
      const { dialog } = createModalShell('GitHub 連携（未設定）');
      const p = document.createElement('p');
      p.style.cssText = 'margin:0;color:#475569';
      p.textContent = 'このデプロイ先ではまだ GitHub App のシークレットが設定されていません。Cloudflare Workers に GITHUB_APP_CLIENT_ID / GITHUB_APP_CLIENT_SECRET / GITHUB_SESSION_SECRET を設定すると GitHub ログインが有効になります。';
      dialog.append(p);
      return;
    }

    if (!session.loggedIn) {
      location.href = '/auth/github/login';
      return;
    }

    const { overlay, dialog } = createModalShell(`GitHub リポジトリ設定 (@${session.login})`);

    const status = document.createElement('div');
    status.style.cssText = 'font-size:12px;color:#475569';
    dialog.append(status);

    // Primary section: Create a new private repository (default: soraneDocs)
    const createBox = document.createElement('form');
    createBox.style.cssText = 'background:#f5f8ff;border:1px solid #d5e2fa;border-radius:10px;padding:12px 14px;display:flex;flex-direction:column;gap:8px';
    createBox.innerHTML = `
      <div style="font-size:13px;font-weight:700;color:#17233d">新しい Private リポジトリを作成（おすすめ）</div>
      <div style="font-size:12px;color:#475569">メモや文書を安全に保存するため、非公開（Private）リポジトリを OKF 形式の <code>index.md</code> 付きで作成します。</div>
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <input name="name" value="soraneDocs" required placeholder="リポジトリ名" style="flex:1;min-width:180px;padding:8px 10px;border:1px solid #cbd5e1;border-radius:8px;background:#fff;font:inherit" />
        <button type="submit" style="padding:8px 14px;border:0;border-radius:8px;background:#1e50b5;color:#fff;font-weight:700;cursor:pointer;white-space:nowrap">Private 作成して接続</button>
      </div>
    `;
    dialog.append(createBox);

    const existingHeading = document.createElement('div');
    existingHeading.textContent = 'または既存のリポジトリを選択';
    existingHeading.style.cssText = 'font-size:12px;font-weight:700;color:#475569;margin-top:2px';
    dialog.append(existingHeading);

    const searchInput = document.createElement('input');
    searchInput.type = 'search';
    searchInput.placeholder = 'リポジトリを検索 (例: owner/repo)';
    searchInput.style.cssText = 'width:100%;padding:8px 10px;border:1px solid #cbd5e1;border-radius:8px;font:inherit';
    dialog.append(searchInput);

    const repoList = document.createElement('div');
    repoList.style.cssText = 'display:flex;flex-direction:column;gap:6px;max-height:190px;overflow:auto;border:1px solid #e2e8f0;border-radius:8px;padding:8px';
    dialog.append(repoList);

    const manualForm = document.createElement('form');
    manualForm.style.cssText = 'display:grid;grid-template-columns:1fr 1fr;gap:8px;border-top:1px solid #e2e8f0;padding-top:10px';
    manualForm.innerHTML = `
      <input name="repo" placeholder="owner/repo を直接指定" required style="grid-column:1/-1;padding:8px 10px;border:1px solid #cbd5e1;border-radius:8px;font:inherit" />
      <input name="branch" placeholder="ブランチ (省略時は既定)" style="padding:8px 10px;border:1px solid #cbd5e1;border-radius:8px;font:inherit" />
      <input name="rootPath" placeholder="サブフォルダ (任意: docs)" style="padding:8px 10px;border:1px solid #cbd5e1;border-radius:8px;font:inherit" />
      <button type="submit" style="grid-column:1/-1;padding:8px 14px;border:1px solid #cbd5e1;border-radius:8px;background:#f8fafc;color:#0f172a;font-weight:700;cursor:pointer">指定したリポジトリを設定</button>
    `;
    if (session.vault) {
      manualForm.elements.repo.value = `${session.vault.owner}/${session.vault.repo}`;
      manualForm.elements.branch.value = session.vault.branch || '';
      manualForm.elements.rootPath.value = session.vault.rootPath || '';
    }
    dialog.append(manualForm);

    const footer = document.createElement('div');
    footer.style.cssText = 'display:flex;flex-wrap:wrap;gap:8px;justify-content:space-between;align-items:center;border-top:1px solid #e2e8f0;padding-top:10px;font-size:12px';

    const leftActions = document.createElement('div');
    leftActions.style.cssText = 'display:flex;gap:8px;flex-wrap:wrap;align-items:center';
    if (session.installUrl) {
      const addRepo = document.createElement('a');
      addRepo.href = session.installUrl;
      addRepo.target = '_blank';
      addRepo.rel = 'noopener noreferrer';
      addRepo.textContent = '許可リポジトリを追加 ↗';
      addRepo.style.cssText = 'color:#1e50b5;font-weight:600;text-decoration:none';
      leftActions.append(addRepo);
    }
    if (session.vault) {
      const toggleBtn = document.createElement('button');
      toggleBtn.type = 'button';
      toggleBtn.textContent = showingRepo ? 'サンプル一覧を表示' : 'リポジトリのファイルを表示';
      toggleBtn.style.cssText = 'border:1px solid #cbd5e1;background:#fff;border-radius:6px;padding:4px 10px;cursor:pointer;font:inherit';
      toggleBtn.onclick = () => {
        showingRepo = !showingRepo;
        applyStartViewConfig();
        overlay.remove();
      };
      leftActions.append(toggleBtn);
    }

    const logoutBtn = document.createElement('button');
    logoutBtn.type = 'button';
    logoutBtn.textContent = 'ログアウト';
    logoutBtn.style.cssText = 'border:1px solid #fecaca;background:#fff5f5;color:#b91c1c;border-radius:6px;padding:4px 10px;cursor:pointer;font:inherit';
    logoutBtn.onclick = async () => {
      await fetch('/auth/github/logout', { method: 'POST', credentials: 'same-origin' });
      session = { ...session, loggedIn: false, login: null, vault: null };
      repoItems = [];
      openedRepoFile = null;
      applyStartViewConfig();
      overlay.remove();
    };
    footer.append(leftActions, logoutBtn);
    dialog.append(footer);

    function showStatus(msg, isError = false, linkUrl = null, linkLabel = null) {
      status.innerHTML = '';
      status.style.color = isError ? '#b91c1c' : '#475569';
      status.append(document.createTextNode(msg));
      if (linkUrl) {
        status.append(document.createTextNode(' '));
        const a = document.createElement('a');
        a.href = linkUrl;
        a.target = '_blank';
        a.rel = 'noopener noreferrer';
        a.textContent = linkLabel || 'GitHub で開く ↗';
        a.style.cssText = 'color:#1e50b5;font-weight:700';
        status.append(a);
      }
    }

    async function selectRepo(owner, repo, branch, rootPath) {
      showStatus(`${owner}/${repo} を設定しています…`);
      const res = await fetch('/api/github/vault', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ owner, repo, branch, rootPath }),
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        showStatus(data.error || `設定に失敗しました (HTTP ${res.status})`, true);
        return;
      }
      session.vault = data.vault;
      showingRepo = true;
      await refreshRepoTree();
      overlay.remove();
    }

    createBox.onsubmit = async (e) => {
      e.preventDefault();
      const repoName = createBox.elements.name.value.trim() || 'soraneDocs';
      showStatus(`Private リポジトリ ${session.login}/${repoName} を作成しています…`);
      const res = await fetch('/api/github/repos/create', {
        method: 'POST',
        credentials: 'same-origin',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: repoName }),
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        showStatus(
          data.error || `作成に失敗しました (HTTP ${res.status})`,
          true,
          data.fallbackCreateUrl || null,
          'GitHub で Private リポジトリを作成 ↗',
        );
        return;
      }
      session.vault = data.vault;
      showingRepo = true;
      await refreshRepoTree();
      overlay.remove();
    };

    manualForm.onsubmit = (e) => {
      e.preventDefault();
      const raw = manualForm.elements.repo.value.trim();
      const parts = raw.split('/');
      if (parts.length !== 2 || !parts[0] || !parts[1]) {
        showStatus('owner/repo 形式で入力してください。', true);
        return;
      }
      void selectRepo(parts[0], parts[1], manualForm.elements.branch.value.trim(), manualForm.elements.rootPath.value.trim());
    };

    let allRepos = [];
    function renderRepos(filterText) {
      repoList.innerHTML = '';
      const q = filterText.trim().toLowerCase();
      const filtered = allRepos.filter((r) => !q || r.fullName.toLowerCase().includes(q));
      if (!filtered.length) {
        const empty = document.createElement('div');
        empty.style.cssText = 'color:#64748b;font-size:12px;padding:6px';
        empty.textContent = allRepos.length
          ? '一致するリポジトリがありません。'
          : '許可されたリポジトリがありません。上のフォームで Private リポジトリを作成するか、「許可リポジトリを追加」から選んでください。';
        repoList.append(empty);
        return;
      }
      for (const r of filtered) {
        const row = document.createElement('button');
        row.type = 'button';
        row.style.cssText = 'display:flex;justify-content:space-between;align-items:center;width:100%;text-align:left;padding:8px 10px;border:1px solid #e2e8f0;border-radius:8px;background:#f8fafc;cursor:pointer;font:inherit';
        const left = document.createElement('span');
        left.style.fontWeight = '600';
        left.textContent = r.fullName;
        const right = document.createElement('span');
        right.style.cssText = 'font-size:11px;color:#64748b';
        right.textContent = `${r.defaultBranch}${r.private ? ' · Private' : ' · Public'}`;
        row.append(left, right);
        row.onclick = () => {
          void selectRepo(r.owner, r.repo, r.defaultBranch, manualForm.elements.rootPath.value.trim());
        };
        repoList.append(row);
      }
    }

    searchInput.oninput = () => renderRepos(searchInput.value);

    showStatus('リポジトリ一覧を読み込んでいます…');
    try {
      const res = await fetch('/api/github/repos', { credentials: 'same-origin' });
      const data = await res.json();
      if (res.ok && Array.isArray(data.repos)) {
        allRepos = data.repos;
        showStatus(`${allRepos.length} 件の既存リポジトリにアクセスできます。`);
        renderRepos('');
      } else {
        showStatus(data.error || 'リポジトリ一覧の取得に失敗しました。', true);
      }
    } catch {
      showStatus('リポジトリ一覧の取得中にネットワークエラーが発生しました。', true);
    }
  }

  window.addEventListener('sorane:start-action', (event) => {
    if (event.detail?.action === 'github-vault') {
      void openRepoModal();
    }
  });

  window.addEventListener('sorane:github-file-opened', (event) => {
    openedRepoFile = event.detail || null;
  });

  // Intercept save/download when a GitHub repository file is open so the user can commit directly back to GitHub.
  window.addEventListener('ibisoffice:download', async (event) => {
    if (!openedRepoFile || !session.loggedIn || !session.vault) return;
    const bytes = event.detail?.bytes;
    if (!(bytes instanceof Uint8Array)) return;
    const message = window.prompt(
      `GitHub (${session.vault.owner}/${session.vault.repo}) へコミットするメッセージを入力してください。\n（キャンセルするとコミットしません）`,
      `Update ${openedRepoFile.path}`,
    );
    if (!message || !message.trim()) return;
    const headers = { 'X-Commit-Message': message.trim() };
    if (openedRepoFile.etag) headers['If-Match'] = openedRepoFile.etag;
    const res = await fetch(`/api/github/file?path=${encodeURIComponent(openedRepoFile.path)}`, {
      method: 'PUT',
      credentials: 'same-origin',
      headers,
      body: bytes,
    });
    const data = await res.json().catch(() => ({}));
    if (res.ok && data.sha) {
      openedRepoFile.etag = `"${data.sha}"`;
      alert(`GitHub にコミットしました (${data.commitSha?.slice(0, 7) || 'ok'})`);
    } else {
      alert(`GitHub コミットに失敗しました: ${data.error || `HTTP ${res.status}`}`);
    }
  });

  void loadSession();
}
