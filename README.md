# TownRise Launcher

TownRise: 시장의 시대 전용 런처입니다.

- Tauri + SvelteKit + TypeScript + Rust
- 픽셀풍 Minecraft 감성 UI
- manifest 기반 파일 업데이트
- SHA-256/size 검증 후 설치

## 개발 실행

```bash
npm install
npm run dev
npm run tauri dev
```

## Windows 실행파일 받기

GitHub repo의 `Actions` 탭에서 `Build Windows Launcher` 워크플로를 수동 실행하면 `townrise-launcher-windows` artifact가 생성됩니다.

1. GitHub repo → `Actions`
2. `Build Windows Launcher`
3. `Run workflow`
4. 완료 후 artifact 다운로드
5. 압축 해제 후 `.msi` 또는 `.exe` 실행

## 검증

```bash
npm run check
npm run build
cd src-tauri
cargo fmt --check
cargo test
cargo check
```

## 현재 범위

현재 MVP는 업데이트 확인/설치 흐름을 구현합니다. Microsoft 로그인과 실제 Minecraft 실행은 다음 단계에서 구현합니다.
