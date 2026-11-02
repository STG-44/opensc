param(
    [Parameter(Position = 0)]
    [string]$GameDir = $env:SHADOW_GAME_DIR,
    [string]$Model,
    [string]$Overrides
)
$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
$localConfig = Join-Path $projectDir 'local/game-dir.txt'
if ([string]::IsNullOrEmpty($GameDir) -and (Test-Path -LiteralPath $localConfig)) {
    $GameDir = (Get-Content -LiteralPath $localConfig -Raw -Encoding UTF8).TrimEnd([char[]]"`r`n")
}
if ([string]::IsNullOrEmpty($GameDir)) {
    throw 'Specify the original game directory: ./scripts/demo-model.ps1 "C:\Games\Shadow Company"'
}
$cargoArguments = @('run', '--locked', '-p', 'shadow-model-viewer', '--', '--game-dir', $GameDir)
if (-not [string]::IsNullOrEmpty($Model)) { $cargoArguments += @('--model', $Model) }
if (-not [string]::IsNullOrEmpty($Overrides)) { $cargoArguments += @('--overrides', $Overrides) }
Push-Location -LiteralPath $projectDir
try {
    & cargo @cargoArguments
    $result = $LASTEXITCODE
}
finally { Pop-Location }
exit $result
