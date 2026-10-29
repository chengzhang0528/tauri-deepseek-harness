param(
    [Parameter(Mandatory = $true)][string]$OldMSI,
    [Parameter(Mandatory = $true)][string]$NewMSI
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') {
    throw 'This installation test is restricted to a disposable GitHub-hosted runner.'
}

function Invoke-Msi([string]$operation, [string]$package) {
    $nativePackage = (Resolve-Path -LiteralPath $package).Path
    $process = Start-Process -FilePath 'msiexec.exe' -ArgumentList @($operation, ('"' + $nativePackage + '"'), '/qn', '/norestart', 'AUTOLAUNCHAPP=0') -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "MSI operation $operation failed with exit code $($process.ExitCode)." }
}

$installRoot = Join-Path $env:LOCALAPPDATA 'DSH Desktop'
$stateMarker = Join-Path $installRoot 'state/ci-upgrade-marker.txt'
$userMarker = Join-Path $env:APPDATA 'DSH Desktop/dsh-home/ci-user-data.txt'
$installed = $false
$currentMsi = $OldMSI
try {
    Invoke-Msi '/i' $OldMSI
    $installed = $true
    if (-not (Get-ChildItem -LiteralPath $installRoot -Filter '*.exe' -File)) { throw 'Installed launcher is missing.' }
    New-Item -ItemType Directory -Force -Path (Split-Path $stateMarker), (Split-Path $userMarker) | Out-Null
    Set-Content -LiteralPath $stateMarker -Value 'state before upgrade' -Encoding utf8
    Set-Content -LiteralPath $userMarker -Value 'user data before upgrade' -Encoding utf8
    Invoke-Msi '/i' $NewMSI
    $currentMsi = $NewMSI
    if (-not (Test-Path -LiteralPath $stateMarker) -or -not (Test-Path -LiteralPath $userMarker)) { throw 'Upgrade removed existing state or user data.' }
    if (-not (Get-ChildItem -LiteralPath $installRoot -Filter '*.exe' -File)) { throw 'Upgraded launcher is missing.' }
    Invoke-Msi '/x' $NewMSI
    $installed = $false
    if (Test-Path -LiteralPath $stateMarker) { throw 'Uninstall left installer-owned state behind.' }
    if (-not (Test-Path -LiteralPath $userMarker)) { throw 'Uninstall removed user data.' }
    if (Test-Path -LiteralPath $installRoot) {
        if (Get-ChildItem -LiteralPath $installRoot -Filter '*.exe' -File) { throw 'Uninstall left the launcher behind.' }
    }
    Write-Output 'Install, upgrade, user-data preservation and uninstall passed.'
} finally {
    if ($installed) { Invoke-Msi '/x' $currentMsi }
}
