# Changelog · Журнал изменений

Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versions follow [SemVer](https://semver.org/).

## [Unreleased]
### Added · Добавлено
- Several monitors with different wallpapers: "Add to favorites" and "Don't show again" ask which screen (tray menu), the settings window has ♥ and 🚫 buttons on each monitor's thumbnail, and the hotkeys act on the monitor under the mouse. Hiding a wallpaper changes only that monitor. · Избранное и «не показывать» для каждого монитора отдельно.
- Hotkey **Win+Alt+X** — don't show this wallpaper again (`hotkey_ban`). · Горячая клавиша «не показывать».
- An installed copy turns on "Start with Windows" on its first run (the portable one doesn't). · Установленная копия сама включает автозапуск.

### Changed · Изменено
- Clicking several categories in a row starts one wallpaper pick after you stop (1.2 s), not one per click. If a category has nothing new, Hora tries another selected one before the fallback. · Один подбор после серии щелчков; при пустой категории — другая.

### Fixed · Исправлено
- Leftovers of an installed update are also cleaned up during the daily update check, not only at the next start. · Остатки обновления убираются и при ежедневной проверке.

## [0.7.0] — 2026-11-14
### Added · Добавлено
- 16 new wallpaper categories (34 in total), including **ASCII art**, pixel art, cyberpunk, synthwave, low poly, flowers, waterfalls, northern lights, planets, Japan and cats. · 16 новых категорий обоев, в том числе ASCII-арт.
- Global hotkeys: **Win+Alt+N** — next wallpaper, **Win+Alt+H** — add to favorites, **Win+Alt+O** — open settings (change them in `config.ini`: `hotkey_next`, `hotkey_fav`, `hotkey_open`; a combination taken by another program is marked in Settings → Other). · Горячие клавиши.
- Pixel scoreboard: soft **glow** around the dots and an optional **blinking colon** (also for Block pixels). · Свечение точек и мигающее двоеточие.
- The settings window remembers its size, position and maximized state. · Окно настроек запоминает размер и положение.
- Two new clock styles: **Pixel scoreboard** (LED dots) and **Block pixels** (chunky squares). · Стили «Пиксельное табло» и «Блочный пиксель».
- The settings window can be resized and maximized; on wide screens the content stays in a centered column. · Окно настроек можно растягивать и разворачивать.
- Test that every Russian label has an English translation (199 strings in code, 67 menu items). · Тест полноты английского перевода.
- "Versions and compatibility" section in the README: SemVer, config compatibility, signed updates. · Раздел «Версии и совместимость».

### Changed · Изменено
- Clock styles have short, clear names ("Big time", "Split letters", "Analog"…) — the preview shows what each looks like. · Понятные названия стилей.

### Fixed · Исправлено
- Crash in USER32 (DrawText) when the settings window drew an empty string — e.g. the Weather page while weather data wasn't loaded. Crash reports now also include the accessed address and the call stack inside Hora. · Падение в USER32 из-за пустой строки в окне настроек; отчёт о падении подробнее.
- Nature, Mountains and other photo categories showed game and digital art: the tag is now required and game/digital art is excluded. When Wallhaven is unreachable, the fallback takes only cached pictures of the selected categories (and never your own folder unless the source includes it). Narrow categories no longer report "nothing found" because of a random empty page. · Природа/Горы больше не показывают игры.
- Changing categories while a wallpaper was being picked was ignored; now Hora picks again with the new categories. · Смена категорий во время подбора больше не теряется.
- A wallpaper, weather or update result that arrived while Hora was busy could get stuck. · Результаты больше не застревают.
- Resizing the settings window could pile up thumbnails in memory. · Память при растягивании окна.
- `--silent` now works in any position (`HoraSetup.exe --all-users --silent`). · Ключи тихой установки в любом порядке.
- Pixel styles ignored their own fonts when a font was picked in Appearance (e.g. Michroma) and were drawn without pixels. Now the style always brings its font; Block pixels get a hard offset shadow. · Пиксельные стили всегда рисуются своим шрифтом; у блочного — жёсткая тень.
- Crash when the settings window was minimized after it became resizable (the clock preview got a negative width). · Падение при свёрнутом окне настроек.
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

[Unreleased]: https://github.com/dark-knight-soul/hora/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/dark-knight-soul/hora/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/dark-knight-soul/hora/compare/v0.5.1...v0.6.0
[0.5.1]: https://github.com/dark-knight-soul/hora/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/dark-knight-soul/hora/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/dark-knight-soul/hora/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/dark-knight-soul/hora/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/dark-knight-soul/hora/releases/tag/v0.2.0
