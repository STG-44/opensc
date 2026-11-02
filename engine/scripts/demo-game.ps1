param(
    [Parameter(Position = 0)]
    [string]$GameDir = $env:SHADOW_GAME_DIR,
    [string]$Overrides,
    [ValidateSet('ru', 'en')]
    [string]$Language,
    [ValidatePattern('^\d+x\d+$')]
    [string]$WindowSize
)
$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
$localConfig = Join-Path $projectDir 'local/game-dir.txt'
if ([string]::IsNullOrEmpty($GameDir) -and (Test-Path -LiteralPath $localConfig)) {
    $GameDir = (Get-Content -LiteralPath $localConfig -Raw -Encoding UTF8).TrimEnd([char[]]"`r`n")
}
if ([string]::IsNullOrEmpty($GameDir)) { throw 'Specify the original game directory: ./scripts/demo-game.ps1 "C:\Games\Shadow Company"' }
$cargoArguments = @('run', '--locked', '-p', 'shadow-model-viewer', '--bin', 'shadow-game', '--', '--game-dir', $GameDir)
if (-not [string]::IsNullOrEmpty($Overrides)) { $cargoArguments += @('--overrides', $Overrides) }
if (-not [string]::IsNullOrEmpty($Language)) { $cargoArguments += @('--language', $Language.ToLowerInvariant()) }
if (-not [string]::IsNullOrEmpty($WindowSize)) { $cargoArguments += @('--window', $WindowSize) }
Push-Location -LiteralPath $projectDir
try {
    & cargo @cargoArguments
    $result = $LASTEXITCODE
}
finally { Pop-Location }
exit $result
