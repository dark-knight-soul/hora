# Changelog · Журнал изменений

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [SemVer](https://semver.org/).

## [0.5.1] — 2026-11-13
### Added · Добавлено
- Silent install and uninstall for winget and admins: `HoraSetup.exe --silent [--all-users]`, `Hora.exe --uninstall --silent`. · Тихая установка и удаление без окон.
- Automatic build and tests on GitHub Actions. · Автосборка и тесты на GitHub.

## [0.5.0] — 2026-11-13
### Added · Добавлено
- Two new pixel fonts with Latin and Cyrillic: **Hora Pixel** (LED dots, like a scoreboard) and **Hora Block** (chunky block pixels). · Два новых пиксельных шрифта с кириллицей: «табло» из точек и блочный.
- Version info in the exe (Properties → Details; Task Manager shows "Hora"). · Версия в свойствах exe.
- Contributing guide, security policy, issue templates, this changelog. · Правила участия, политика безопасности, шаблоны задач, журнал изменений.

### Changed · Изменено
- LICENSE is now the plain MIT text (font licenses stay in `assets/fonts/OFL-*.txt`), so GitHub recognizes it. · LICENSE — чистый текст MIT.

## [0.4.0] — 2026-11-13
### Added
- One-click updates from GitHub Releases, verified with an ECDSA P-256 signature. · Обновления в один щелчок с проверкой подписи.

## [0.3.0] — 2026-11-13
### Added
- English interface for settings, tray menu and installer. · Английский интерфейс.

## [0.2.0] — 2026-11-13
### Added
- First public release: wallpapers from Wallhaven or a folder, 12 clock styles, weather, favorites and blacklist, live wallpapers (effects and video, incl. Wallpaper Engine video), installer and portable version. · Первый публичный выпуск.

[0.5.1]: https://github.com/dark-knight-soul/hora/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/dark-knight-soul/hora/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/dark-knight-soul/hora/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/dark-knight-soul/hora/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/dark-knight-soul/hora/releases/tag/v0.2.0
