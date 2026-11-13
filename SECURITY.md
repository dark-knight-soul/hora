# Security policy · Безопасность

## Supported versions
Only the [latest release](https://github.com/dark-knight-soul/hora/releases/latest) gets security fixes. Hora updates itself (Settings → Other), so please stay on it.

## Reporting a vulnerability · Как сообщить об уязвимости
Please **do not** open a public issue. Use GitHub's private reporting instead:
**Security → Report a vulnerability** on the repository page.
Пожалуйста, не создавайте публичную задачу — используйте «Security → Report a vulnerability».

You should get an answer within 7 days. Include the Hora version, Windows version and steps to reproduce.

## How updates are protected · Как защищены обновления
- Hora downloads updates only over HTTPS from this repository's GitHub Releases.
- An update is installed only if `HoraSetup.exe.sig` is a valid ECDSA P-256 signature made with the project's private key; the public key is built into the program (`src/update.rs`).
- The exe restricts DLL loading to System32 (protection against a planted DLL next to the exe).
