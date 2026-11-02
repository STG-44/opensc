param(
    [Parameter(Position = 0)]
    [string]$GameDir = $env:SHADOW_GAME_DIR,
    [string]$Overrides
)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'demo-map.ps1') -GameDir $GameDir -Overrides $Overrides -Player
exit $LASTEXITCODE
