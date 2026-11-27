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
    if (session.loggedIn && session.vault && repoItems.length > 0) {
      const label = `${session.vault.owner}/${session.vault.repo}`;
      const relPaths = repoItems.map((f) => f.subtitle);
      window.ibisOfficeSetWebVaultTree?.(label, relPaths);
      const isFormAssistDir = (p) => /^(profiles|forms|requests|credentials)\//i.test(p);
      const textItems = repoItems
        .filter((f) => /\.(md|markdown|lawtext|txt|sd-jwt|mdoc)$/i.test(f.name))
        .sort((a, b) => Number(isFormAssistDir(b.subtitle)) - Number(isFormAssistDir(a.subtitle)))
        .slice(0, 60);
      void (async () => {
        for (const item of textItems) {
          try {
            const r = await fetch(item.url, { credentials: 'same-origin' });
            if (r.ok) {
              const text = await r.text();
              window.ibisOfficeHydrateWebVaultFile?.(item.subtitle, text);
            }
          } catch {
            /* ignore transient fetch failures */
          }
        }
      })();
    }
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

    // Guided banner shown when the GitHub App is not yet installed or lacks repo creation approval.
    const installBanner = document.createElement('div');
    installBanner.style.cssText = 'display:none;background:#fffbeb;border:1px solid #fcd34d;border-radius:10px;padding:12px 14px;flex-direction:column;gap:8px';
    dialog.append(installBanner);

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
        if (res.status === 401 || data.reauthRequired) {
          showStatus(
            data.error || 'GitHub の認証が期限切れです。再ログインしてください。',
            true,
            '/auth/github/login',
            'GitHub で再ログイン ↗',
          );
          return;
        }
        if (data.initFailed) {
          showStatus(
            data.error || `初期ファイルの書き込みに失敗しました (HTTP ${res.status})`,
            true,
            data.installSettingsUrl || data.installUrl || data.repoUrl || null,
            data.installSettingsUrl || data.installUrl ? 'GitHub App 設定を開く ↗' : 'GitHub でリポジトリを確認 ↗',
          );
          return;
        }
        const needsInstall = data.diagnostics?.installation === null && (data.installUrl || session.installUrl);
        const settingsOrFallback = data.installSettingsUrl || data.fallbackCreateUrl || null;
        showStatus(
          data.error || `作成に失敗しました (HTTP ${res.status})`,
          true,
          needsInstall ? (data.installUrl || session.installUrl) : settingsOrFallback,
          needsInstall
            ? 'GitHub App をインストール ↗'
            : (data.installSettingsUrl ? 'GitHub App 設定を開く ↗' : 'GitHub で Private リポジトリを作成 ↗'),
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

    let needsInstallRefresh = false;

    function updateInstallBanner(installed, canCreateRepo, installHref, repoSelection, installSettingsUrl, hasSoraneDocsRepo) {
      const href = installHref || session.installUrl;
      if (installed === false && href) {
        needsInstallRefresh = true;
        installBanner.style.display = 'flex';
        installBanner.innerHTML = '';
        const heading = document.createElement('div');
        heading.style.cssText = 'font-size:13px;font-weight:700;color:#92400e';
        heading.textContent = 'ステップ 1: GitHub App（soraneDocs）をインストール';
        const desc = document.createElement('div');
        desc.style.cssText = 'font-size:12px;color:#78350f';
        desc.textContent = 'リポジトリの自動作成やファイルの読み書きを行うには、ご自身の GitHub アカウントへのインストールが必要です（「All repositories」を選択してください）。インストール後にこの画面へ戻ると自動で反映されます。';
        const actions = document.createElement('div');
        actions.style.cssText = 'display:flex;gap:8px;flex-wrap:wrap;align-items:center';
        const installLink = document.createElement('a');
        installLink.href = href;
        installLink.target = '_blank';
        installLink.rel = 'noopener noreferrer';
        installLink.textContent = 'GitHub App をインストール ↗';
        installLink.style.cssText = 'display:inline-block;padding:7px 14px;border-radius:8px;background:#1e50b5;color:#fff;font-weight:700;text-decoration:none;font-size:13px';
        const reloadBtn = document.createElement('button');
        reloadBtn.type = 'button';
        reloadBtn.textContent = 'インストール後に再読み込み';
        reloadBtn.style.cssText = 'padding:7px 12px;border:1px solid #d97706;border-radius:8px;background:#fff;color:#92400e;font-weight:600;cursor:pointer;font-size:12px';
        reloadBtn.onclick = () => { void loadRepos(); };
        actions.append(installLink, reloadBtn);
        installBanner.append(heading, desc, actions);
        return;
      }
      if (installed === true && canCreateRepo === false && !hasSoraneDocsRepo) {
        needsInstallRefresh = true;
        installBanner.style.display = 'flex';
        installBanner.innerHTML = '';
        const heading = document.createElement('div');
        heading.style.cssText = 'font-size:13px;font-weight:700;color:#92400e';
        heading.textContent = repoSelection === 'selected'
          ? 'リポジトリ自動作成には「All repositories」の許可が必要です'
          : 'リポジトリ自動作成の権限承認が必要です';
        const desc = document.createElement('div');
        desc.style.cssText = 'font-size:12px;color:#78350f';
        desc.textContent = repoSelection === 'selected'
          ? '現在「Only select repositories」に設定されているため、新規作成した Private リポジトリを自動初期化できません。GitHub のアプリ設定で「All repositories」に変更し、権限（Repository creation: Write）の承認状況を確認してください。'
          : '新しい Private リポジトリを自動作成するには、GitHub のアプリ設定画面で権限更新（Repository creation: Write）を承認してください。';
        const actions = document.createElement('div');
        actions.style.cssText = 'display:flex;gap:8px;flex-wrap:wrap;align-items:center';
        const settingsLink = document.createElement('a');
        settingsLink.href = installSettingsUrl || 'https://github.com/settings/installations';
        settingsLink.target = '_blank';
        settingsLink.rel = 'noopener noreferrer';
        settingsLink.textContent = 'GitHub でアプリ設定を開く ↗';
        settingsLink.style.cssText = 'display:inline-block;padding:7px 14px;border-radius:8px;background:#1e50b5;color:#fff;font-weight:700;text-decoration:none;font-size:13px';
        const reloadBtn = document.createElement('button');
        reloadBtn.type = 'button';
        reloadBtn.textContent = '設定後に再読み込み';
        reloadBtn.style.cssText = 'padding:7px 12px;border:1px solid #d97706;border-radius:8px;background:#fff;color:#92400e;font-weight:600;cursor:pointer;font-size:12px';
        reloadBtn.onclick = () => { void loadRepos(); };
        actions.append(settingsLink, reloadBtn);
        installBanner.append(heading, desc, actions);
        return;
      }
      needsInstallRefresh = false;
      installBanner.style.display = 'none';
      installBanner.innerHTML = '';
    }

    async function loadRepos() {
      showStatus('リポジトリ一覧を読み込んでいます…');
      try {
        const res = await fetch('/api/github/repos', { credentials: 'same-origin' });
        const data = await res.json().catch(() => ({}));
        if (res.status === 401 || data.reauthRequired) {
          showStatus(
            data.error || 'GitHub の認証が期限切れです。再ログインしてください。',
            true,
            '/auth/github/login',
            'GitHub で再ログイン ↗',
          );
          return;
        }
        if (res.ok && Array.isArray(data.repos)) {
          allRepos = data.repos;
          const loginLower = String(session.login || '').toLowerCase();
          const hasSoraneDocsRepo = allRepos.some(
            (r) => r.owner?.toLowerCase() === loginLower && r.repo?.toLowerCase() === 'soranedocs',
          );
          createBox.style.display = hasSoraneDocsRepo ? 'none' : 'flex';
          existingHeading.textContent = hasSoraneDocsRepo
            ? '接続するリポジトリを選択'
            : 'または既存のリポジトリを選択';
          updateInstallBanner(
            data.installed,
            data.canCreateRepo,
            data.installUrl,
            data.repoSelection,
            data.installSettingsUrl,
            hasSoraneDocsRepo,
          );
          if (data.installed === false) {
            showStatus('GitHub App がまだインストールされていません。上のボタンからインストールしてください。');
          } else {
            showStatus(`${allRepos.length} 件の既存リポジトリにアクセスできます。`);
          }
          renderRepos(searchInput.value);
        } else {
          showStatus(data.error || 'リポジトリ一覧の取得に失敗しました。', true);
        }
      } catch {
        showStatus('リポジトリ一覧の取得中にネットワークエラーが発生しました。', true);
      }
    }

    const onWindowFocus = () => {
      if (!document.body.contains(overlay)) {
        window.removeEventListener('focus', onWindowFocus);
        return;
      }
      if (needsInstallRefresh) {
        void loadRepos();
      }
    };
    window.addEventListener('focus', onWindowFocus);

    await loadRepos();
  }

  window.addEventListener('sorane:start-action', (event) => {
    if (event.detail?.action === 'github-vault') {
      void openRepoModal();
    }
  });

  window.addEventListener('sorane:github-file-opened', (event) => {
    openedRepoFile = event.detail || null;
  });

  async function saveToGitHub(name, bytes) {
    if (!session.loggedIn || !session.vault) {
      return { handled: false, name: null };
    }
    if (!(bytes instanceof Uint8Array)) {
      return { handled: false, name: null };
    }
    const existingPath = openedRepoFile?.path || null;
    let targetPath = existingPath;
    if (!targetPath) {
      const suggested = name || 'Untitled.md';
      const chosen = window.prompt(
        `GitHub (${session.vault.owner}/${session.vault.repo}) へ新規ファイルとして保存するパスを入力してください。\n（キャンセルするとローカル保存になります）`,
        suggested,
      );
      if (!chosen || !chosen.trim()) {
        return { handled: false, name: null };
      }
      targetPath = chosen.trim().replace(/^\/+/, '');
    }
    const message = window.prompt(
      `GitHub (${session.vault.owner}/${session.vault.repo}) へコミットするメッセージを入力してください。\n（キャンセルするとコミットしません）`,
      `${existingPath ? 'Update' : 'Add'} ${targetPath}`,
    );
    if (!message || !message.trim()) {
      return { handled: Boolean(existingPath), name: null };
    }
    const headers = { 'X-Commit-Message': message.trim() };
    if (openedRepoFile?.etag) headers['If-Match'] = openedRepoFile.etag;
    const res = await fetch(`/api/github/file?path=${encodeURIComponent(targetPath)}`, {
      method: 'PUT',
      credentials: 'same-origin',
      headers,
      body: bytes,
    });
    const data = await res.json().catch(() => ({}));
    if (res.ok && data.sha) {
      const savedName = targetPath.split('/').pop() || targetPath;
      openedRepoFile = { path: targetPath, name: savedName, etag: `"${data.sha}"` };
      void refreshRepoTree();
      return { handled: true, name: savedName };
    }
    const errMsg = data.error || `HTTP ${res.status}`;
    alert(`GitHub コミットに失敗しました: ${errMsg}`);
    throw new Error(errMsg);
  }

  window.soraneSaveToGitHub = saveToGitHub;

  // Keep supporting ibisoffice:download if dispatched by embed/export callers.
  window.addEventListener('ibisoffice:download', (event) => {
    const bytes = event.detail?.bytes;
    if (!(bytes instanceof Uint8Array)) return;
    void saveToGitHub(event.detail?.name || 'Untitled.md', bytes).then((result) => {
      if (result.handled && result.name) {
        alert(`GitHub にコミットしました (${result.name})`);
      }
    }).catch(() => {});
  });

  let activeAiAbortController = null;

  function normalizeBaseUrl(raw, fallback) {
    const trimmed = (raw || '').trim().replace(/\/+$/, '');
    return trimmed || fallback;
  }

  function isLikelyNonChatModel(id) {
    const lower = id.toLowerCase();
    return (
      lower.includes('embedding') ||
      lower.includes('whisper') ||
      lower.includes('tts') ||
      lower.includes('dall-e') ||
      lower.includes('moderation') ||
      lower.includes('transcribe') ||
      lower.includes('realtime') ||
      lower.includes('image') ||
      lower.includes('audio')
    );
  }

  function isFastModel(id) {
    const lower = id.toLowerCase();
    return (
      lower.includes('mini') ||
      lower.includes('flash') ||
      lower.includes('haiku') ||
      lower.includes('nano') ||
      lower.includes('lite') ||
      lower.includes('small')
    );
  }

  async function readSseLines(body, onLine) {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      let nl;
      while ((nl = buffer.indexOf('\n')) !== -1) {
        const line = buffer.slice(0, nl).replace(/\r$/, '');
        buffer = buffer.slice(nl + 1);
        if (line.length > 0 && typeof onLine === 'function') {
          onLine(line);
        }
      }
    }
    if (buffer.trim().length > 0 && typeof onLine === 'function') {
      onLine(buffer.replace(/\r$/, ''));
    }
  }

  window.ibisOfficeAiAbort = () => {
    if (activeAiAbortController) {
      activeAiAbortController.abort();
      activeAiAbortController = null;
    }
  };

  window.ibisOfficeAiModels = async () => {
    let res;
    try {
      res = await fetch('/api/ai/models', { credentials: 'same-origin' });
    } catch {
      throw new Error('network_error');
    }
    if (res.status === 401) {
      throw new Error('login_required');
    }
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      throw new Error(data.error || 'ai_failed');
    }
    return await res.text();
  };

  window.ibisOfficeAiStream = async (body, csrf, onLine) => {
    window.ibisOfficeAiAbort();
    const controller = new AbortController();
    activeAiAbortController = controller;
    try {
      let res;
      try {
        res = await fetch('/api/ai/chat', {
          method: 'POST',
          credentials: 'same-origin',
          headers: {
            'Content-Type': 'application/json',
            'X-Sorane-CSRF': csrf || '',
          },
          body,
          signal: controller.signal,
        });
      } catch (err) {
        if (err && err.name === 'AbortError') throw err;
        throw new Error('network_error');
      }
      if (!res.ok) {
        const data = await res.json().catch(() => ({}));
        throw new Error(data.error || (res.status === 401 ? 'login_required' : 'ai_failed'));
      }
      if (!res.body) {
        throw new Error('ai_failed');
      }
      await readSseLines(res.body, onLine);
    } catch (err) {
      if (err && err.name === 'AbortError') {
        throw new Error('aborted');
      }
      throw err;
    } finally {
      if (activeAiAbortController === controller) {
        activeAiAbortController = null;
      }
    }
  };

  window.ibisOfficeAiDirectModels = async (provider, baseUrl, apiKey) => {
    const kind = (provider || 'openai').trim().toLowerCase();
    const key = (apiKey || '').trim();
    const defaultBase =
      kind === 'anthropic'
        ? 'https://api.anthropic.com/v1'
        : kind === 'gemini'
          ? 'https://generativelanguage.googleapis.com/v1beta/openai'
          : 'https://api.openai.com/v1';
    const base = normalizeBaseUrl(baseUrl, defaultBase).replace(/\/chat\/completions$/, '').replace(/\/messages$/, '');
    const url = `${base}/models`;
    const headers = { Accept: 'application/json' };
    if (kind === 'anthropic') {
      if (key) headers['x-api-key'] = key;
      headers['anthropic-version'] = '2023-06-01';
      headers['anthropic-dangerous-direct-browser-access'] = 'true';
    } else if (key) {
      headers.Authorization = `Bearer ${key}`;
    }

    let res;
    try {
      res = await fetch(url, { method: 'GET', headers });
    } catch {
      throw new Error('network_error');
    }
    if (!res.ok) {
      const data = await res.json().catch(() => ({}));
      const msg = data?.error?.message || data?.message || `HTTP ${res.status}`;
      throw new Error(res.status === 401 ? 'invalid_api_key' : msg);
    }
    const payload = await res.json().catch(() => ({}));
    const rawList = Array.isArray(payload?.data)
      ? payload.data
      : Array.isArray(payload?.models)
        ? payload.models
        : Array.isArray(payload)
          ? payload
          : [];
    const seen = new Set();
    const models = [];
    for (const item of rawList) {
      const rawId = typeof item?.id === 'string' ? item.id : typeof item?.name === 'string' ? item.name : '';
      const id = rawId.trim().replace(/^models\//, '');
      if (!id || seen.has(id) || isLikelyNonChatModel(id)) continue;
      seen.add(id);
      const display = typeof item?.display_name === 'string' && item.display_name.trim()
        ? `${item.display_name.trim()} (${id})`
        : id;
      models.push({
        key: id,
        label: display,
        maxInputChars: 32000,
        fast: isFastModel(id),
      });
    }
    const defaultModel = models.find((m) => !m.fast)?.key || models[0]?.key || null;
    const completionModel = models.find((m) => m.fast)?.key || models[0]?.key || null;
    return JSON.stringify({
      csrf: '',
      provider: kind,
      models,
      defaultModel,
      completionModel,
    });
  };

  window.ibisOfficeAiDirectStream = async (provider, baseUrl, apiKey, bodyJson, onLine) => {
    window.ibisOfficeAiAbort();
    const controller = new AbortController();
    activeAiAbortController = controller;
    try {
      const kind = (provider || 'openai').trim().toLowerCase();
      const key = (apiKey || '').trim();
      const parsed = JSON.parse(bodyJson);
      const defaultBase =
        kind === 'anthropic'
          ? 'https://api.anthropic.com/v1'
          : kind === 'gemini'
            ? 'https://generativelanguage.googleapis.com/v1beta/openai'
            : 'https://api.openai.com/v1';
      const base = normalizeBaseUrl(baseUrl, defaultBase);

      let url;
      let headers;
      let payload;
      if (kind === 'anthropic') {
        url = base.endsWith('/messages') ? base : `${base}/messages`;
        headers = {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream',
          'anthropic-version': '2023-06-01',
          'anthropic-dangerous-direct-browser-access': 'true',
          ...(key ? { 'x-api-key': key } : {}),
        };
        const systemParts = [];
        const turns = [];
        for (const msg of parsed.messages || []) {
          if (msg.role === 'system' && msg.content) {
            systemParts.push(msg.content);
          } else if ((msg.role === 'user' || msg.role === 'assistant') && msg.content) {
            turns.push({ role: msg.role, content: msg.content });
          }
        }
        payload = JSON.stringify({
          model: parsed.model,
          max_tokens: 4096,
          stream: true,
          ...(systemParts.length ? { system: systemParts.join('\n\n') } : {}),
          messages: turns,
        });
      } else {
        url = base.endsWith('/chat/completions') ? base : `${base}/chat/completions`;
        headers = {
          'Content-Type': 'application/json',
          Accept: 'text/event-stream',
          ...(key ? { Authorization: `Bearer ${key}` } : {}),
        };
        payload = JSON.stringify({
          model: parsed.model,
          messages: parsed.messages,
          stream: true,
        });
      }

      let res;
      try {
        res = await fetch(url, {
          method: 'POST',
          headers,
          body: payload,
          signal: controller.signal,
        });
      } catch (err) {
        if (err && err.name === 'AbortError') throw err;
        throw new Error('network_error');
      }
      if (!res.ok) {
        const errData = await res.json().catch(() => ({}));
        const detail = errData?.error?.message || errData?.message || '';
        if (res.status === 401) throw new Error('invalid_api_key');
        if (res.status === 429) throw new Error('rate_limited');
        throw new Error(detail || `HTTP ${res.status}`);
      }
      if (!res.body) {
        throw new Error('ai_failed');
      }
      if (kind === 'anthropic') {
        await readSseLines(res.body, (line) => {
          const trimmed = line.trim();
          if (!trimmed.startsWith('data:')) return;
          const data = trimmed.slice(5).trim();
          if (!data) return;
          let ev;
          try {
            ev = JSON.parse(data);
          } catch {
            return;
          }
          if (ev.type === 'content_block_delta' && ev.delta?.type === 'text_delta' && typeof ev.delta.text === 'string') {
            onLine(`data: ${JSON.stringify({ choices: [{ delta: { content: ev.delta.text } }] })}`);
          } else if (ev.type === 'message_stop') {
            onLine('data: [DONE]');
          } else if (ev.type === 'error') {
            onLine(`data: ${JSON.stringify({ error: { message: ev.error?.message || 'Anthropic API error' } })}`);
          }
        });
      } else {
        await readSseLines(res.body, onLine);
      }
    } catch (err) {
      if (err && err.name === 'AbortError') {
        throw new Error('aborted');
      }
      throw err;
    } finally {
      if (activeAiAbortController === controller) {
        activeAiAbortController = null;
      }
    }
  };

  void loadSession();
}
