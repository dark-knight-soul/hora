# Contributing · Как помочь проекту

Thanks for your interest in Hora! Issues and pull requests in English or Russian are welcome.
Спасибо за интерес к Hora! Задачи и запросы на слияние можно писать по-английски или по-русски.

## Reporting a bug · Сообщить об ошибке
1. Check that you are on the [latest release](https://github.com/dark-knight-soul/hora/releases/latest).
2. Open an issue with the **Bug report** template: Windows version, number of monitors and scaling, what you did, what you expected and what happened.
3. Attach `hora.log` (tray menu → Open log) — remove anything personal from it first (city, folder paths).

## Building · Сборка
- Windows 10/11 x64, Rust stable (the `x86_64-pc-windows-gnu` toolchain works without Visual Studio).
- `powershell -ExecutionPolicy Bypass -File build.ps1` — builds `dist\Hora.exe` (portable).
- `cargo test --release` — unit tests.

## Pull requests
- One topic per PR; describe what changed and how you tested it (a screenshot for UI changes).
- Keep the program light: no heavy new dependencies, no CPU use when idle, nothing sent to the network beyond what the README lists under *Privacy*.
- Code comments are in Russian (project convention); English is fine in PR descriptions.
- Every new user-visible string goes into `src/i18n.rs` with an English translation.
- By submitting a PR you agree that your contribution is licensed under the [MIT License](LICENSE).

## Releases (maintainer) · Выпуск
Bump `version` in `Cargo.toml`, add a section to `CHANGELOG.md`, run `package.ps1` (it signs the installer), then publish a GitHub release with `HoraSetup.exe`, `HoraSetup.exe.sig`, `Hora-portable.zip`, `SHA256SUMS.txt`.
