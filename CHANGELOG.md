# Changelog · Журнал изменений

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [SemVer](https://semver.org/).

## [Unreleased]
### Added · Добавлено
- Global hotkeys: **Win+Alt+N** — next wallpaper, **Win+Alt+H** — add to favorites (change them in `config.ini`: `hotkey_next`, `hotkey_fav`; a combination taken by another program is marked in Settings → Other). · Горячие клавиши.
- Pixel scoreboard: soft **glow** around the dots and an optional **blinking colon** (also for Block pixels). · Свечение точек и мигающее двоеточие.
- The settings window remembers its size, position and maximized state. · Окно настроек запоминает размер и положение.
- Two new clock styles: **Pixel scoreboard** (LED dots) and **Block pixels** (chunky squares). · Стили «Пиксельное табло» и «Блочный пиксель».
- The settings window can be resized and maximized; on wide screens the content stays in a centered column. · Окно настроек можно растягивать и разворачивать.
- Test that every Russian label has an English translation (199 strings in code, 67 menu items). · Тест полноты английского перевода.
- "Versions and compatibility" section in the README: SemVer, config compatibility, signed updates. · Раздел «Версии и совместимость».

### Changed · Изменено
- Clock styles have short, clear names ("Big time", "Split letters", "Analog"…) — the preview shows what each looks like. · Понятные названия стилей.

### Fixed · Исправлено
- "All users" updates: the elevation request always waits for the UAC answer, fresh update files are never cleaned up mid-update, and the log says whether rights were declined or the replacement failed. · Обновление «для всех»: надёжнее ожидание UAC, понятный журнал.
- The installer window title is now translated ("Hora Setup"), and its question reads more naturally in English. · Заголовок установщика переведён.
- Updating after a long time no longer goes step by step (0.4 → 0.5 → 0.6): right before installing, Hora asks GitHub again and installs the newest version. · Обновление больше не идёт по ступенькам: перед установкой Hora перепроверяет последнюю версию.

## [0.6.0] — 2026-11-14
### Added · Добавлено
- Crash handling: the reason goes to `crash.txt`, Hora restarts itself once (not more often than every 2 minutes) and offers to report the error on GitHub with a prefilled form. Nothing is sent automatically. · Перехват падений: причина в `crash.txt`, один автоперезапуск, предложение сообщить об ошибке.
- Settings → Other → **Report a problem**: opens the bug form with version and Windows filled in. · Кнопка «Сообщить о проблеме».
- Config compatibility test: `config.ini` files from all previous versions keep working. · Тест совместимости `config.ini`.

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

[Unreleased]: https://github.com/dark-knight-soul/hora/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/dark-knight-soul/hora/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/dark-knight-soul/hora/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/dark-knight-soul/hora/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/dark-knight-soul/hora/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/dark-knight-soul/hora/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/dark-knight-soul/hora/releases/tag/v0.2.0
