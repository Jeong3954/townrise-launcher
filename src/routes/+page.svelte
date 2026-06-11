<script lang="ts">
  import { checkForUpdates, installUpdates, launchGame, type InstallSummary, type UpdatePlan } from '$lib/tauri';

  type LauncherState = 'idle' | 'checking' | 'ready' | 'updating' | 'updated' | 'error';

  let state: LauncherState = 'idle';
  let plan: UpdatePlan | null = null;
  let summary: InstallSummary | null = null;
  let message = '광장 문이 열릴 준비를 하고 있습니다.';

  const stateLabel: Record<LauncherState, string> = {
    idle: '대기 중',
    checking: '확인 중',
    ready: '준비됨',
    updating: '업데이트 중',
    updated: '최신 상태',
    error: '확인 필요'
  };

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
    message = '업데이트를 설치하고 있습니다. 잠시만 기다려 주세요.';
    try {
      summary = await installUpdates();
      state = 'updated';
      message = '업데이트가 완료되었습니다.';
      plan = await checkForUpdates();
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  async function onLaunch() {
    try {
      await launchGame();
    } catch (error) {
      state = 'error';
      message = friendlyError(error);
    }
  }

  function friendlyError(error: unknown) {
    const text = error instanceof Error ? error.message : String(error);
    if (text.includes('not implemented')) return '게임 실행은 다음 단계에서 연결됩니다. 지금은 업데이트 기능을 먼저 준비했습니다.';
    if (text.includes('network') || text.includes('request')) return '업데이트 서버에 연결하지 못했습니다. 잠시 후 다시 시도해 주세요.';
    return '작업을 완료하지 못했습니다. 다시 시도해 주세요.';
  }

  function mb(bytes: number) {
    if (!bytes) return '0 MB';
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

<svelte:head>
  <title>TownRise Launcher</title>
</svelte:head>

<main class="launcher-shell">
  <section class="hero pixel-panel">
    <div class="logo-block" aria-hidden="true">
      <div class="grass"></div>
      <div class="dirt"></div>
      <div class="stone"></div>
    </div>

    <div class="hero-copy">
      <p class="eyebrow">TownRise: 시장의 시대</p>
      <h1>마을의 하루를<br />시작하세요</h1>
      <p class="intro">필요한 파일을 간단히 확인하고, 준비가 끝나면 바로 서버로 입장합니다.</p>
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
        <button class="pixel-button secondary" on:click={onCheck} disabled={state === 'checking' || state === 'updating'}>
          {state === 'checking' ? '확인 중...' : '업데이트 확인'}
        </button>
        <button class="pixel-button" on:click={onInstall} disabled={!plan?.updateRequired || state === 'updating'}>
          {state === 'updating' ? '설치 중...' : '업데이트 설치'}
        </button>
        <button class="pixel-button primary" on:click={onLaunch} disabled={state === 'updating'}>게임 시작</button>
      </div>
    </div>

    <div class="pixel-panel update-panel">
      <h2>패치 상태</h2>
      {#if plan}
        <div class="simple-row"><span>버전</span><strong>{plan.version}</strong></div>
        <div class="simple-row"><span>필요 용량</span><strong>{mb(plan.totalDownloadSize)}</strong></div>
        <div class="progress-track"><span style={`width: ${plan.updateRequired ? 42 : 100}%`}></span></div>
        <p class="hint">{plan.updateRequired ? '업데이트 설치를 누르면 필요한 파일만 받습니다.' : '필요한 파일이 모두 준비되어 있습니다.'}</p>
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
        <li>TownRise 전용 런처 초안이 준비되었습니다.</li>
        <li>업데이트는 시작 전에 안전하게 검증됩니다.</li>
        <li>게임 실행 연결은 다음 단계에서 진행됩니다.</li>
      </ul>
    </div>

    <div class="pixel-panel server-panel">
      <h2>서버 상태</h2>
      <div class="server-line"><span class="online"></span><strong>준비 중</strong></div>
      <p class="hint">정식 서버 상태 연동 전까지는 간단 상태만 표시합니다.</p>
    </div>
  </section>
</main>
