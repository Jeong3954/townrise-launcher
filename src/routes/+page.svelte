<script lang="ts">
  import { onMount } from 'svelte';
  import {
    beginMicrosoftLogin,
    checkForUpdates,
    closeWindow,
    currentMinecraftSession,
    installLauncherSelfUpdate,
    installUpdates,
    launchGame,
    logoutMinecraft,
    minimizeWindow,
    onMinecraftBootstrapProgress,
    onUpdateProgress,
    openExternalUrl,
    pollMicrosoftLogin,
    startWindowDrag,
    type InstallSummary,
    type LauncherSelfUpdateProgress,
    type LoginStart,
    type MinecraftSession,
    type UpdatePlan,
    type UpdateProgress
  } from '$lib/tauri';

  type LauncherState = 'idle' | 'checking' | 'ready' | 'updating' | 'updated' | 'launching' | 'login' | 'error';
  type ScreenState = 'booting' | 'updating' | 'login' | 'main' | 'error';

  let screen: ScreenState = 'booting';
  let state: LauncherState = 'checking';
  let plan: UpdatePlan | null = null;
  let summary: InstallSummary | null = null;
  let progress: UpdateProgress | null = null;
  let authSession: MinecraftSession | null = null;
  let loginStart: LoginStart | null = null;
  let loginPolling = false;
  let message = 'TownRise 런처 업데이트를 확인하고 있습니다.';

  const stateLabel: Record<LauncherState, string> = {
    idle: '대기 중',
    checking: '확인 중',
    ready: '준비됨',
    updating: '업데이트 중',
    updated: '최신 상태',
    launching: '실행 중',
    login: '로그인 필요',
    error: '확인 필요'
  };

  onMount(() => {
    let cleanup: () => void = () => undefined;
    let cleanupBootstrap: () => void = () => undefined;
    onUpdateProgress((next) => {
      progress = next;
      message = progressMessage(next);
    }).then((unlisten) => {
      cleanup = unlisten;
    });
    onMinecraftBootstrapProgress((next) => {
      progress = next;
      message = minecraftProgressMessage(next);
    }).then((unlisten) => {
      cleanupBootstrap = unlisten;
    });

    runStartupUpdate();
    return () => {
      cleanup();
      cleanupBootstrap();
    };
  });

  async function runStartupUpdate() {
    screen = 'booting';
    state = 'checking';
    message = '런처 업데이트를 확인하고 있습니다.';
    summary = null;
    progress = null;
    try {
      const launcherUpdated = await installLauncherSelfUpdate((next) => {
        message = launcherSelfUpdateMessage(next);
      });
      if (launcherUpdated) return;

      message = '실행 전에 필요한 업데이트를 확인하고 있습니다.';
      plan = await checkForUpdates();
      if (!plan.updateRequired) {
        authSession = await currentMinecraftSession();
        if (!authSession) {
          state = 'login';
          message = 'Microsoft 계정으로 로그인하면 정품 Minecraft 세션으로 실행합니다.';
          screen = 'login';
          return;
        }

        state = 'updated';
        message = '최신 상태입니다. 바로 입장할 수 있습니다.';
        screen = 'main';
        return;
      }

      screen = 'updating';
      state = 'updating';
      message = '업데이트 파일을 설치하고 있습니다.';
      summary = await installUpdates();
      plan = await checkForUpdates();
      progress = {
        phase: 'finished',
        currentFile: null,
        completedFiles: plan.files.length,
        totalFiles: plan.files.length,
        downloadedBytes: summary.totalBytes,
        totalBytes: summary.totalBytes,
        percent: 100
      };
      state = 'updated';
      message = '업데이트가 완료되었습니다.';
      authSession = await currentMinecraftSession();
      if (!authSession) {
        state = 'login';
        message = '업데이트가 완료되었습니다. Microsoft 계정으로 로그인해 주세요.';
        screen = 'login';
        return;
      }
      screen = 'main';
    } catch (error) {
      try {
        authSession = await currentMinecraftSession();
        if (!authSession) {
          state = 'login';
          screen = 'login';
          message = '업데이트 확인은 완료하지 못했지만, 먼저 Microsoft 로그인을 진행할 수 있습니다.';
          return;
        }
      } catch {
        // Fall through to the normal friendly error screen.
      }
      state = 'error';
      screen = 'error';
      message = friendlyError(error);
    }
  }

  async function onCheck() {
    state = 'checking';
    message = '필요한 파일을 확인하고 있습니다.';
    summary = null;
    try {
      plan = await checkForUpdates();
      state = plan.updateRequired ? 'ready' : 'updated';
      message = plan.updateRequired ? '새로운 TownRise 파일이 준비되어 있습니다.' : '최신 상태입니다. 바로 입장할 수 있습니다.';
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  async function onInstall() {
    state = 'updating';
    screen = 'updating';
    message = '업데이트를 설치하고 있습니다. 잠시만 기다려 주세요.';
    try {
      summary = await installUpdates();
      state = 'updated';
      message = '업데이트가 완료되었습니다.';
      plan = await checkForUpdates();
      authSession = await currentMinecraftSession();
      screen = authSession ? 'main' : 'login';
      if (!authSession) {
        state = 'login';
        message = 'Microsoft 계정으로 로그인해 주세요.';
      }
    } catch (error) {
      state = 'error';
      screen = 'error';
      message = friendlyError(error);
    }
  }

  async function onBeginLogin() {
    screen = 'login';
    state = 'login';
    loginPolling = false;
    message = 'Microsoft 로그인 코드를 요청하고 있습니다.';
    try {
      loginStart = await beginMicrosoftLogin();
      message = '브라우저에서 Microsoft 로그인을 완료해 주세요.';
      pollLoginUntilComplete(loginStart.deviceCode, loginStart.interval);
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  async function onOpenLoginUrl() {
    if (!loginStart) return;
    const url = loginStart.verificationUriComplete ?? loginStart.verificationUri;
    try {
      await openExternalUrl(url);
      message = '브라우저에서 Microsoft 로그인을 완료해 주세요.';
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  async function pollLoginUntilComplete(deviceCode: string, intervalSeconds: number) {
    loginPolling = true;
    let waitSeconds = Math.max(2, intervalSeconds);
    while (loginPolling) {
      await new Promise((resolve) => setTimeout(resolve, waitSeconds * 1000));
      try {
        const result = await pollMicrosoftLogin(deviceCode);
        if (result === 'Pending') {
          message = '아직 승인 대기 중입니다. Microsoft 로그인 창을 확인해 주세요.';
          continue;
        }
        if (result === 'SlowDown') {
          waitSeconds += 5;
          message = 'Microsoft 요청 제한으로 잠시 후 다시 확인합니다.';
          continue;
        }
        authSession = result.Complete;
        loginPolling = false;
        state = 'updated';
        message = `${authSession.username} 계정으로 로그인되었습니다.`;
        screen = 'main';
      } catch (error) {
        loginPolling = false;
        state = 'error';
        message = friendlyError(error);
      }
    }
  }

  async function onLogout() {
    await logoutMinecraft();
    authSession = null;
    loginStart = null;
    state = 'login';
    message = '로그아웃했습니다. 다시 Microsoft 계정으로 로그인해 주세요.';
    screen = 'login';
  }

  async function onLaunch() {
    if (!authSession) {
      state = 'login';
      screen = 'login';
      message = '게임 시작 전에 Microsoft 계정으로 로그인해 주세요.';
      return;
    }
    try {
      state = 'launching';
      progress = {
        phase: 'checking',
        currentFile: 'Minecraft 실행 파일',
        completedFiles: 0,
        totalFiles: 1,
        downloadedBytes: 0,
        totalBytes: 0,
        percent: 0
      };
      message = 'Minecraft 실행 파일을 준비하고 있습니다. 첫 실행은 파일 다운로드 때문에 시간이 걸릴 수 있습니다.';
      const pid = await launchGame();
      state = 'updated';
      message = `Minecraft 프로세스를 시작했습니다. PID ${pid}`;
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  function friendlyError(error: unknown) {
    const text = error instanceof Error ? error.message : String(error);
    if (text.includes('Microsoft login client id')) return 'Microsoft 로그인 앱 ID가 아직 설정되지 않았습니다. 런처 데이터 폴더의 microsoft-client-id.txt 또는 TOWNRISE_MS_CLIENT_ID로 설정해야 합니다.';
    if (text.includes('does not own Minecraft')) return '이 Microsoft 계정에는 Minecraft Java Edition 보유 기록이 없습니다.';
    if (text.includes('Microsoft login is required')) return '게임 시작 전에 Microsoft 로그인이 필요합니다.';
    if (text.includes('Minecraft profile is missing')) return 'Minecraft Java 프로필 이름이 없습니다. 공식 런처에서 프로필을 먼저 만들어 주세요.';
    if (text.includes('launch config is missing')) {
      return 'Minecraft 실행 설정을 만들지 못했습니다. Java 설치와 네트워크 상태를 확인해 주세요.';
    }
    if (text.includes('metadata') || text.includes('bootstrap') || text.includes('download hash mismatch')) return 'Minecraft 실행 파일 준비에 실패했습니다. 네트워크 상태를 확인한 뒤 다시 시도해 주세요.';
    if (text.includes('failed to start Minecraft process')) return 'Minecraft 실행에 실패했습니다. PC에 Java 21 이상이 설치되어 있는지 확인해 주세요.';
    if (text.includes('network') || text.includes('request') || text.includes('manifest request')) return '업데이트 서버에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.';
    if (text.includes('hash mismatch') || text.includes('size mismatch')) return '업데이트 파일 검증에 실패했습니다. 안전을 위해 실행하지 않았습니다.';
    return '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
  }

  function progressMessage(next: UpdateProgress) {
    if (next.phase === 'checking') return `${next.currentFile ?? '파일'} 상태를 확인하고 있습니다.`;
    if (next.phase === 'downloading') return `${next.currentFile ?? '파일'} 다운로드 중입니다.`;
    if (next.phase === 'installed') return `${next.currentFile ?? '파일'} 설치 완료.`;
    if (next.phase === 'finished') return '업데이트 마무리 중입니다.';
    return '업데이트를 준비하고 있습니다.';
  }

  function minecraftProgressMessage(next: UpdateProgress) {
    if (next.phase === 'checking') return `${next.currentFile ?? 'Minecraft 파일'} 확인 중입니다.`;
    if (next.phase === 'downloading') return `${next.currentFile ?? 'Minecraft 파일'} 다운로드 중입니다.`;
    if (next.phase === 'installed') return `${next.currentFile ?? 'Minecraft 파일'} 준비 완료.`;
    if (next.phase === 'finished') return 'Minecraft 실행 준비가 완료되었습니다.';
    return 'Minecraft 실행 파일을 준비하고 있습니다.';
  }

  function launcherSelfUpdateMessage(next: LauncherSelfUpdateProgress) {
    if (next.phase === 'checking') return '런처 새 버전을 확인하고 있습니다.';
    if (next.phase === 'downloading') {
      return next.version ? `런처 ${next.version} 버전을 다운로드하고 있습니다.` : '런처 업데이트를 다운로드하고 있습니다.';
    }
    if (next.phase === 'installing') return '런처 업데이트를 설치하고 있습니다.';
    if (next.phase === 'restarting') return '런처 업데이트가 완료되어 다시 시작합니다.';
    return '런처가 최신 상태입니다.';
  }

  function mb(bytes: number) {
    if (!bytes) return '0 MB';
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }

  $: visibleProgress = progress?.percent ?? (screen === 'booting' ? 12 : 0);
  $: mainPanelProgress = state === 'launching' ? visibleProgress : plan?.updateRequired ? 42 : 100;
</script>

<svelte:head>
  <title>TownRise Launcher</title>
</svelte:head>

<div class="app-window">
  <div class="custom-titlebar" role="toolbar" tabindex="-1" aria-label="창 조작" data-tauri-drag-region on:mousedown={startWindowDrag}>
    <div class="window-brand" data-tauri-drag-region>
      <span class="mini-logo" aria-hidden="true"><span></span><span></span><span></span></span>
      <strong data-tauri-drag-region>TownRise Launcher</strong>
    </div>
    <div class="window-actions">
      <button class="window-button minimize" aria-label="최소화" on:pointerdown|stopPropagation on:mousedown|stopPropagation on:click|stopPropagation={minimizeWindow}>—</button>
      <button class="window-button close" aria-label="닫기" on:pointerdown|stopPropagation on:mousedown|stopPropagation on:click|stopPropagation={closeWindow}>×</button>
    </div>
  </div>

  {#if screen === 'booting' || screen === 'updating'}
    <main class="launcher-shell update-only-shell">
      <section class="pixel-panel update-gate">
        <div class="logo-block update-logo" aria-hidden="true">
          <div class="grass"></div>
          <div class="dirt"></div>
          <div class="stone"></div>
        </div>
        <div>
          <p class="eyebrow">TownRise 업데이트</p>
          <h1>{screen === 'booting' ? '입장 준비 중' : '새 파일 설치 중'}</h1>
          <p class="intro">{message}</p>
        </div>
        <div class="progress-track large"><span style={`width: ${visibleProgress}%`}></span></div>
        <div class="update-meta">
          <span>{progress?.completedFiles ?? 0} / {progress?.totalFiles ?? plan?.files.length ?? 0} 파일</span>
          <strong>{visibleProgress}%</strong>
        </div>
      </section>
    </main>
  {:else if screen === 'login'}
    <main class="launcher-shell update-only-shell">
      <section class="pixel-panel login-gate">
        <div class="logo-block update-logo" aria-hidden="true">
          <div class="grass"></div>
          <div class="dirt"></div>
          <div class="stone"></div>
        </div>
        <div>
          <p class="eyebrow">Microsoft 로그인</p>
          <h1>정품 계정으로<br />입장 준비</h1>
          <p class="intro">{message}</p>
        </div>
        {#if loginStart}
          <div class="login-code-card">
            <span>입력 코드</span>
            <strong>{loginStart.userCode}</strong>
            <p>{loginStart.verificationUri} 에 접속해 코드를 입력하세요.</p>
          </div>
          <button class="pixel-button primary login-link" on:click={onOpenLoginUrl}>Microsoft 로그인 열기</button>
          <p class="hint">로그인이 끝나면 런처가 자동으로 계정 확인을 완료합니다.</p>
        {:else}
          <button class="pixel-button primary" on:click={onBeginLogin}>Microsoft 계정으로 로그인</button>
          <p class="hint">Microsoft OAuth → Xbox Live/XSTS → Minecraft Services 순서로 정품 Java 프로필을 확인합니다.</p>
        {/if}
      </section>
    </main>
  {:else}
    <main class="launcher-shell">
      <div class="launcher-scroll">
        <section class="hero pixel-panel">
          <div class="logo-block" aria-hidden="true">
            <div class="grass"></div>
            <div class="dirt"></div>
            <div class="stone"></div>
          </div>

          <div class="hero-copy">
            <p class="eyebrow">TownRise: 시장의 시대</p>
            <h1>마을의 하루,<br />곧 시작됩니다</h1>
            <p class="intro">필요한 파일만 빠르게 확인하고, 준비가 끝나면 바로 서버로 입장합니다.</p>
          </div>

          <div class="status-card">
            <span class={`status-dot ${state}`}></span>
            <div>
              <strong>{stateLabel[state]}</strong>
              <p>{message}</p>
            </div>
          </div>
        </section>

        <section class="content-grid">
          <div class="pixel-panel control-panel">
            <h2>입장 준비</h2>
            <div class="button-stack">
              <button class="pixel-button secondary" on:click={onCheck} disabled={state === 'checking' || state === 'updating' || state === 'launching'}>
                {state === 'checking' ? '확인 중...' : '업데이트 다시 확인'}
              </button>
              <button class="pixel-button" on:click={onInstall} disabled={!plan?.updateRequired || state === 'updating' || state === 'launching'}>
                {state === 'updating' ? '설치 중...' : '업데이트 설치'}
              </button>
              <button class="pixel-button primary" on:click={onLaunch} disabled={!authSession || state === 'updating' || state === 'launching'}>{state === 'launching' ? '실행 중...' : '게임 시작'}</button>
            </div>
          </div>

          <div class="pixel-panel update-panel">
            <h2>패치 상태</h2>
            {#if plan}
              <div class="simple-row"><span>버전</span><strong>{plan.version}</strong></div>
              <div class="simple-row"><span>필요 용량</span><strong>{state === 'launching' ? `${progress?.completedFiles ?? 0} / ${progress?.totalFiles ?? 1} 파일` : mb(plan.totalDownloadSize)}</strong></div>
              <div class="progress-track"><span style={`width: ${mainPanelProgress}%`}></span></div>
              <p class="hint">{state === 'launching' ? message : plan.updateRequired ? '업데이트 설치를 누르면 필요한 파일만 받습니다.' : '필요한 파일이 모두 준비되어 있습니다.'}</p>
            {:else}
              <p class="hint">먼저 업데이트 확인을 눌러 현재 상태를 확인하세요.</p>
            {/if}
            {#if summary}
              <p class="success">{summary.installed}개 파일 설치 완료</p>
            {/if}
          </div>

          <div class="pixel-panel notice-panel">
            <h2>오늘의 소식</h2>
            <ul>
              <li>런처 시작 시 업데이트를 먼저 확인합니다.</li>
              <li>업데이트가 있으면 전용 업데이트 화면에서 설치합니다.</li>
              <li>게임 시작 시 Minecraft 1.21.1 실행 파일을 자동 준비한 뒤 실행합니다.</li>
            </ul>
          </div>

          <div class="pixel-panel server-panel">
            <h2>계정</h2>
            {#if authSession}
              <div class="server-line"><span class="online"></span><strong>{authSession.username}</strong></div>
              <p class="hint">Microsoft/Minecraft 세션으로 게임을 실행합니다.</p>
              <button class="pixel-button secondary compact" on:click={onLogout}>로그아웃</button>
            {:else}
              <p class="hint">게임 시작 전에 Microsoft 로그인이 필요합니다.</p>
              <button class="pixel-button primary compact" on:click={onBeginLogin}>Microsoft 로그인</button>
            {/if}
          </div>
        </section>
      </div>
    </main>
  {/if}
</div>
