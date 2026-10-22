param(
    [Parameter(Position = 0)]
    [string]$GameDir = $env:SHADOW_GAME_DIR,
    [string]$Overrides
)

$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
$localConfig = Join-Path $projectDir 'local/game-dir.txt'
if ([string]::IsNullOrEmpty($GameDir) -and (Test-Path -LiteralPath $localConfig)) {
    $GameDir = (Get-Content -LiteralPath $localConfig -Raw -Encoding UTF8).TrimEnd([char[]]"`r`n")
}
if ([string]::IsNullOrEmpty($GameDir)) {
    throw 'Specify the original game directory: ./scripts/demo.ps1 "C:\Games\Shadow Company"'
}

# Pass paths as individual arguments, including spaces and non-ASCII names.
$cargoArguments = @('run', '--locked', '-p', 'shadow-viewer', '--', '--game-dir', $GameDir)
if (-not [string]::IsNullOrEmpty($Overrides)) {
    $cargoArguments += @('--overrides', $Overrides)
}

Push-Location -LiteralPath $projectDir
try {
    & cargo @cargoArguments
    $result = $LASTEXITCODE
}
finally {
    Pop-Location
}
exit $result
