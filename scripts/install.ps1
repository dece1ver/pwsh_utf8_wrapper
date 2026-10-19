<#
.SYNOPSIS
    Установщик pwsh_utf8_wrapper для OpenCode.

.DESCRIPTION
    Скачивает (или собирает из исходников) pwsh.exe и запускает его с --install,
    который копирует бинарник в %USERPROFILE%\.config\opencode\bin и обновляет
    opencode.json (с бэкапом).

.PARAMETER FromSource
    Собрать из исходников текущего репозитория вместо скачивания релиза.

.PARAMETER Version
    Тег релиза (по умолчанию "latest").

.PARAMETER BinDir
    Каталог назначения (по умолчанию %USERPROFILE%\.config\opencode\bin).

.PARAMETER Uninstall
    Удалить ключ "shell" из конфига.

.PARAMETER Help
    Показать справку.

.EXAMPLE
    irm https://raw.githubusercontent.com/dece1ver/pwsh_utf8_wrapper/main/scripts/install.ps1 | iex
#>
param(
    [switch]$FromSource,
    [string]$Version = "latest",
    [string]$BinDir,
    [string]$Config,
    [switch]$Uninstall,
    [switch]$Help
)

if ($Help) {
    Get-Help $MyInvocation.MyCommand.Path -Detailed
    return
}

$ErrorActionPreference = "Stop"

$repo = "dece1ver/pwsh_utf8_wrapper"
$tmp = Join-Path $env:TEMP "pwsh_utf8_wrapper"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

$exe = $null

if ($FromSource) {
    $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
    $root = Resolve-Path (Join-Path $scriptDir "..")
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw "cargo не найден в PATH. Установите Rust (https://rustup.rs) или запустите без -FromSource."
    }
    Push-Location $root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "сборка завершилась с ошибкой" }
    } finally {
        Pop-Location
    }
    $exe = Join-Path $root "target\release\pwsh.exe"
    if (-not (Test-Path $exe)) { throw "бинарник не найден: $exe" }
} else {
    $asset = "pwsh.exe"
    if ($Version -eq "latest") {
        $latest = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases/latest" -UseBasicParsing
        $tag = $latest.tag_name
        $url = "https://github.com/$repo/releases/download/$tag/$asset"
        Write-Host "[install.ps1] последний релиз: $tag"
    } else {
        $url = "https://github.com/$repo/releases/download/$Version/$asset"
    }
    $exe = Join-Path $tmp "pwsh.exe"
    Write-Host "[install.ps1] скачивание: $url"
    Invoke-WebRequest -Uri $url -OutFile $exe -UseBasicParsing
    if (-not (Test-Path $exe)) { throw "не удалось скачать $url" }

    # проверка checksum, если есть
    try {
        $shaUrl = $url + ".sha256"
        $shaFile = Join-Path $tmp "pwsh.exe.sha256"
        Invoke-WebRequest -Uri $shaUrl -OutFile $shaFile -UseBasicParsing -ErrorAction Stop
        $expected = (Get-Content $shaFile | Select-Object -First 1).Split()[0]
        $actual = (Get-FileHash $exe -Algorithm SHA256).Hash
        if ($expected -and ($expected.ToLower() -ne $actual.ToLower())) {
            throw "checksum не совпал: expected $expected, got $actual"
        }
        Write-Host "[install.ps1] checksum OK"
    } catch {
        Write-Host "[install.ps1] checksum недоступен, пропускаем проверку"
    }
}

$argList = @()
if ($Uninstall) { $argList += "--uninstall" } else { $argList += "--install" }
if ($BinDir) { $argList += "--bin-dir"; $argList += $BinDir }
if ($Config) { $argList += "--config"; $argList += $Config }

Write-Host "[install.ps1] запуск: $exe $($argList -join ' ')"
& $exe @argList
if ($LASTEXITCODE -ne 0) {
    throw "установщик завершился с кодом $LASTEXITCODE"
}
