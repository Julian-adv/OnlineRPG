[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Get-TomlString([string]$Value) {
    return $Value.Replace('\', '\\').Replace('"', '\"').Replace("`r", '\r').Replace("`n", '\n')
}

$repo = if ($env:REPO) { $env:REPO } else { Split-Path -Parent $PSScriptRoot }
$repo = (Resolve-Path -LiteralPath $repo).Path
$outDir = if ($env:OUT_DIR) { $env:OUT_DIR } else { Join-Path $repo "dist" }
$serverHost = if ($env:HOST) { $env:HOST } else { "openmmo.to.nexus" }
$clientSecret = $env:GOOGLE_CLI_CLIENT_SECRET

if ([string]::IsNullOrWhiteSpace($clientSecret)) {
    throw "Set GOOGLE_CLI_CLIENT_SECRET (Google Cloud -> the CLI OAuth client)."
}
# v0.2.0 shipped a value with the GO chopped off, and every packaged sign-in
# failed with invalid_client. Refuse to build rather than find out downstream.
if (-not $clientSecret.StartsWith("GOCSPX-")) {
    throw "GOOGLE_CLI_CLIENT_SECRET does not start with GOCSPX-; Google installed-app secrets always do."
}

$target = $env:TARGET
$oldRustFlags = $env:RUSTFLAGS
$stage = $null

Push-Location $repo
try {
    # Named by crate version; the release tag must be agent-client-v<version>
    # and bumped every release (the shipped self-updater compares against it).
    $cargoToml = Get-Content -LiteralPath (Join-Path $repo "agent-client\Cargo.toml") -Raw
    if ($cargoToml -notmatch '(?m)^version = "(.+)"') { throw "no version in agent-client/Cargo.toml" }
    $version = $Matches[1]

    $targetTriple = if ($target) {
        $target
    } else {
        (& rustc -Vv | Select-String '^host:' | ForEach-Object { $_.Line.Substring(5).Trim() })
    }
    if ($targetTriple -notmatch '^(?<arch>.+?)-(?:pc|unknown)-windows-(?<abi>.+)$') {
        throw "Windows target expected, got '$targetTriple'."
    }

    $suffix = "$($Matches.arch)-windows-$($Matches.abi)"
    $name = "agent-client-v$version-$suffix"
    $stage = Join-Path $outDir $name
    $archive = Join-Path $outDir "$name.zip"

    # crt-static is part of cargo's fingerprint, so give it its own target dir:
    # sharing target/ would recompile the whole tree here and again on the next
    # plain `cargo build`.
    $buildDir = Join-Path $repo "target\package-win"
    $targetArgs = if ($target) { @("--target", $target) } else { @() }
    $releaseDir = if ($target) { Join-Path $buildDir "$target\release" } else { Join-Path $buildDir "release" }

    $env:RUSTFLAGS = "$oldRustFlags -C target-feature=+crt-static".Trim()
    & cargo build --release @targetArgs --target-dir $buildDir -p agent-client
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $binary = Join-Path $releaseDir "agent-client.exe"

    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Path (Join-Path $stage "data") -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination $stage
    # No data/templates: those are operator NPC roles (merchant, guard). A user
    # agent layers its own role file over the shared data/system_prompt.txt.
    Copy-Item -LiteralPath @(
        (Join-Path $repo "agent-client\data\system_prompt.txt"),
        (Join-Path $repo "agent-client\data\animation_durations.json")
    ) -Destination (Join-Path $stage "data")
    # Ship both roles to copy from, and start the editable one off as newcomer:
    # a fresh agent that learns the world by playing it.
    New-Item -ItemType Directory -Path (Join-Path $stage "data\user_prompts") -Force | Out-Null
    Copy-Item -Path (Join-Path $repo "agent-client\data\user_prompts\*.txt") `
        -Destination (Join-Path $stage "data\user_prompts")
    Copy-Item -LiteralPath (Join-Path $repo "agent-client\data\user_prompts\newcomer.txt") `
        -Destination (Join-Path $stage "data\user_prompt.txt")

    # Shared with package-agent-client.sh so the shipped config cannot drift.
    # Registry NPC personas are operator-side; a user agent plays its own character.
    $config = (Get-Content -LiteralPath (Join-Path $repo "tools\agent-client-config.toml.in") -Raw).
        Replace('@HOST@', (Get-TomlString $serverHost)).
        Replace('@CLIENT_SECRET@', (Get-TomlString $clientSecret))
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText((Join-Path $stage "data\config.toml"), $config, $utf8)

    Copy-Item -LiteralPath (Join-Path $repo "doc\AGENT_CLIENT_QUICKSTART.md") -Destination (Join-Path $stage "README.md")
    Compress-Archive -LiteralPath $stage -DestinationPath $archive -CompressionLevel Optimal -Force
    Write-Output "==> $archive"
} finally {
    if ($null -ne $stage -and (Test-Path -LiteralPath $stage)) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
    $env:RUSTFLAGS = $oldRustFlags
    Pop-Location
}
