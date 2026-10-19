# pwsh_utf8_wrapper

UTF-8-обёртка над настоящим `pwsh.exe` для shell-инструмента OpenCode на Windows.
Чинит `�` / кракозябры в выводе shell-команд с кириллицей.

## Почему это нужно (кратко)

OpenCode декодирует stdout дочернего `pwsh.exe` жёстко как UTF-8
(`tool/shell.ts:487` → `new TextDecoder()`), а `pwsh` пишет в пайп
в кодировке своей консоли (на ru-RU Windows — CP866). Результат: валидные
внутри PowerShell строки превращаются в `�` по дороге к модели.
Повлиять изнутри сессии нельзя: `-NoProfile` отрезает профиль,
`[Console]::OutputEncoding = UTF8` заблокировано ConstrainedLanguage,
`chcp 65001` в headless-процессе — no-op (всё измерено).

Враппер действует снаружи: выставляет консоли UTF-8 **до** старта настоящего
`pwsh` (`.NET` читает кодовую страницу один раз при старте и кэширует),
затем прозрачно запускает настоящий `pwsh` с теми же аргументами.

## Требования к развёртыванию (важно!)

1. **Имя файла обязано быть `pwsh.exe`.** OpenCode выбирает argv-ветку по
   имени exe (`packages/core/src/shell.ts`, таблица `META`): только
   `pwsh`/`powershell` получают `-NoLogo -NoProfile -NonInteractive -Command`.
   Любое другое имя уводит в ветку `cmd /d /s /c` и ломает контракт.
   Задано в `Cargo.toml` (`[[bin]] name = "pwsh"`).
2. Лежать должен в **отдельном каталоге**, не входящем в `PATH`
   (защита от рекурсии + код сам исключает свой каталог при поиске настоящего pwsh).
3. Настоящий `pwsh` ищется: `PATH` (кроме своего каталога) →
   `%ProgramFiles%\PowerShell\7\pwsh.exe` → известные Store-пути →
   fallback `powershell.exe` 5.1.

## Что чинит / что нет

| Кейс | Результат |
|---|---|
| cmdlet-строки pwsh, `Write-Error`, `Get-Content` UTF-8 | ✅ UTF-8 |
| native-утилиты с UTF-8-выводом | ✅ без изменений (уже работали) |
| native-exe с выхлопом 866/1251 | ❌ останется (лечится только декодером OpenCode) |
| `read`/`grep`/`glob`, ввод кириллицы | ✅ не затрагиваются |

Побочный эффект: меняется кодовая страница **общей** консоли (той вкладки
терминала, где работает opencode) на 65001 — сохраняется до перезапуска
консоли. Для UTF-8-вывода TUI это скорее плюс. Отдельные вкладки терминала
не затрагиваются.

---

## ИНСТРУКЦИЯ ДЛЯ ЧЕЛОВЕКА

### Сборка (Windows, нужен Rust MSVC + `link.exe` из VS Build Tools)

```cmd
cd C:\Users\<you>\rust\pwsh_utf8_wrapper
cargo build --release
```

Результат: `target\release\pwsh.exe` (~100–200 КБ, static CRT —
VC Redist на других машинах не нужен).

### Установка

```cmd
mkdir %USERPROFILE%\.config\opencode\bin
copy target\release\pwsh.exe %USERPROFILE%\.config\opencode\bin\pwsh.exe
```

Бекап конфига, затем в `%USERPROFILE%\.config\opencode\opencode.json` добавить:

```json
"shell": "C:/Users/<you>/.config/opencode/bin/pwsh.exe"
```

(прямые слеши; имя пользователя — своё). Перезапустить сессию opencode.

### Проверка

В shell OpenCode выполнить команду, выводящую `Привет`. Ожидается корректный
русский текст вместо `�`.

### Откат

Удалить строку `"shell"` из `opencode.json`, перезапустить opencode.
При желании вернуть консоли 866: `chcp 866`. Остаточных эффектов нет.

### Перенос на другую машину

Достаточно скопировать `bin\pwsh.exe` и прописать `"shell"` с поправкой
на имя пользователя. Пересборка нужна только при смене архитектуры
(собран x86_64). Настоящий `pwsh` на новой машине найдётся сам
(PATH → ProgramFiles → Store → fallback powershell 5.1).

### Автоустановка (one-liner)

Самый простой путь — скачать и запустить установщик одной командой (PowerShell 7):

```powershell
irm https://raw.githubusercontent.com/dece1ver/pwsh_utf8_wrapper/main/scripts/install.ps1 | iex
```

Установщик скачает последний релиз `pwsh.exe`, положит в
`%USERPROFILE%\.config\opencode\bin\pwsh.exe`, сделает бэкап `opencode.json`
и добавит ключ `"shell"`. После — перезапустите сессию opencode.

Для удаления:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/dece1ver/pwsh_utf8_wrapper/main/scripts/install.ps1))) -Uninstall
```

Или локально из клона репозитория:

```powershell
.\scripts\install.ps1 -FromSource
.\scripts\install.ps1 -Uninstall
```

Сам бинарник тоже умеет устанавливать/удалять себя:

```cmd
%USERPROFILE%\.config\opencode\bin\pwsh.exe --install
%USERPROFILE%\.config\opencode\bin\pwsh.exe --uninstall
%USERPROFILE%\.config\opencode\bin\pwsh.exe --status
%USERPROFILE%\.config\opencode\bin\pwsh.exe --help
```

Опции: `--bin-dir <путь>`, `--config <путь>`, `--dry-run`, `--force`.

### Проверка (до и после)

До установки (или после отката, когда `"shell"` убран из конфига) команда
с кириллицей в stdout приходит с кракозябрами/`�`:

```powershell
# в shell OpenCode, без "shell" в конфиге:
pwsh -NoLogo -NoProfile -NonInteractive -Command "Write-Output 'Привет'"
# ожидается: ��� / мусор вместо «Привет»
```

После установки и перезапуска сессии opencode — корректный текст:

```powershell
# в shell OpenCode, с "shell" в конфиге:
pwsh -NoLogo -NoProfile -NonInteractive -Command "Write-Output 'Привет'"
# ожидается: Привет
```

Проверка exit code (должен пробрасываться 1:1):

```powershell
pwsh -NoLogo -NoProfile -NonInteractive -Command "exit 3"
# ожидается: код выхода 3
```

Headless-стенд (без OpenCode), Node-реплика спавна:

```js
const { spawn } = require('child_process');
const p = spawn(process.env.USERPROFILE + '/.config/opencode/bin/pwsh.exe',
  ['-NoLogo','-NoProfile','-NonInteractive','-Command','Write-Output "Привет"'],
  { stdio: ['ignore','pipe','pipe'] });
p.stdout.on('data', d => console.log('stdout:', d.toString('utf8')));
p.on('exit', c => console.log('exit:', c));
```

Ожидается `stdout: Привет` и `exit: 0`. До фикса в stdout были байты CP866
(`8FE0…`), декодирование давало `�`.

Негативный контроль: временно убрать `"shell"` из `opencode.json`, перезапустить
сессию — баг должен вернуться; вернуть строку обратно.

---

## ИНСТРУКЦИЯ ДЛЯ АГЕНТА (внедрение в opencode)

Контекст: OpenCode 1.18.x, Windows, ru-RU, `[Console]::OutputEncoding=866`
у spawned pwsh, ConstrainedLanguage активен. Ограничения: не менять систему
глобально, не трогать профиль PS, имя бинарника — строго `pwsh.exe`.

> **Важно про перезапуск:** `shell` из конфига opencode читается один раз
> при старте сессии. Поэтому живая проверка (шаг 6) достоверна только после
> перезапуска сессии — вывод shell-команд до рестарта идёт через старый shell
> и ни о чём не говорит. Headless-стенд (шаг 5) работает без рестарта.

### Шаги внедрения

1. **Сборка.** `cargo build --release` в корне проекта. Линковщик:
   `link.exe` из Visual Studio (`vswhere -requires VC.Tools` для поиска),
   окружение — только на время команды сборки (VsDevCmd), систему не менять.
   Приёмка: `target\release\pwsh.exe` существует, запускается, без аргументов
   ведёт себя как `pwsh -NoLogo -NoProfile -NonInteractive -Command ""`
   (открывает… нет: без `-Command` просто пробрасывает пустой argv —
   проверять только через шаги ниже).
2. **Деплой.** Создать `%USERPROFILE%\.config\opencode\bin\`, скопировать
   туда `pwsh.exe`. Убедиться: каталог НЕ в `PATH`; файл не совпадает
   с резолвом настоящего pwsh (рекурсии быть не должно — код исключает
   свой каталог, но проверить вызовом `<bin>\pwsh.exe -NoLogo -NoProfile
   -NonInteractive -Command "$PSVersionTable.PSVersion"` — должен ответить
   настоящий pwsh 7.x).
3. **Бекап конфига.** Скопировать `opencode.json` → `opencode.json.bak-<дата>`.
4. **Правка конфига.** Добавить `"shell": "C:/Users/<user>/.config/opencode/bin/pwsh.exe"`.
   Резолв проверяется кодом OpenCode (`Shell.acceptable` → `stat().isFile()`).
5. **Верификация стендом (без OpenCode).** Прогнать Node-реплику спавна
   OpenCode (`spawn(wrapper, ["-NoLogo","-NoProfile","-NonInteractive",
   "-Command", <cmd>], {stdio:["ignore","pipe","pipe"]})`) с командой,
   выводящей `Привет` через `[char]`-коды. Приёмка: hex stdout начинается
   с `D09F` (UTF-8), декодирование даёт `Привет`. До фикса там `8FE0…` (866).
6. **Живая проверка.** Перезапустить сессию opencode, выполнить shell-команду
   с кириллицей в stdout и в stderr. Приёмка: корректный текст, exit code
   проброшен (проверить неуспешный код тоже, напр. `exit 3` → `exit: 3`).
7. **Конкурентный smoke.** Два параллельных shell-вызова с кириллицей —
   оба корректны (гонок за CP быть не должно: оба выставляют 65001).
8. **Негативный контроль.** Временно убрать `"shell"` → убедиться, что `�`
   вернулись → вернуть строку (доказательство причинности).
9. **Финал.** Доложить: hex до/после, живой вывод, коды выхода, smoke, откат.

### Критерии приёмки (все обязательны)

- [ ] `pwsh -Command <кириллица>` через shell OpenCode возвращает текст, а не `�`
- [ ] stderr с кириллицей тоже цел
- [ ] exit code пробрасывается 1:1 (включая ненулевой)
- [ ] без `"shell"` баг воспроизводится (контроль)
- [ ] `cargo build --release` воспроизводим с нуля (чистый `target/`)

### Запреты

- Не переименовывать бинарник (см. «Требования», п.1).
- Не класть бинарник в каталог из `PATH`.
- Не писать ничего в stdout враппера (только проброс; диагностика — stderr + коды 126/127).
- Не править `opencode.json` без бекапа; не трогать систему/профиль/chcp глобально.
