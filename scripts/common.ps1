Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RepositoryRoot = Split-Path -Parent $PSScriptRoot

function Invoke-NousDeveloperCommand {
    param([Parameter(Mandatory = $true)][string]$Command)

    if ($env:OS -ne "Windows_NT") {
        & sh -c $Command
        if ($LASTEXITCODE -ne 0) { throw "command failed with exit code $LASTEXITCODE" }
        return
    }

    $vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) {
        throw "Visual Studio Build Tools were not found"
    }
    $installation = & $vswhere -latest -products * -property installationPath
    if (-not $installation) { throw "Visual Studio Build Tools installation is unavailable" }
    $rustHost = (& rustc -vV | Select-String '^host:').Line
    $architecture = if ($rustHost -match 'aarch64-pc-windows-msvc') { "arm64" } else { "x64" }
    $toolsRoot = Join-Path $installation "VC\Tools\MSVC"
    $toolset = Get-ChildItem -LiteralPath $toolsRoot -Directory |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "bin\Host$architecture\$architecture\cl.exe") } |
        Sort-Object { [version]$_.Name } -Descending |
        Select-Object -First 1
    if (-not $toolset) { throw "No complete MSVC $architecture compiler toolset was found" }
    $versionParts = $toolset.Name.Split('.')
    $vcvarsVersion = "$($versionParts[0]).$($versionParts[1])"
    $vcvars = Join-Path $installation "VC\Auxiliary\Build\vcvarsall.bat"
    $developerCommand = "call `"$vcvars`" $architecture -vcvars_ver=$vcvarsVersion >nul && $Command"
    & cmd.exe /d /s /c $developerCommand
    if ($LASTEXITCODE -ne 0) { throw "command failed with exit code $LASTEXITCODE" }
}

function Invoke-NousCargo {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    Invoke-NousDeveloperCommand "cargo $([string]::Join(' ', $Arguments))"
}

function Invoke-NousPython {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    & python @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "python command failed with exit code $LASTEXITCODE"
    }
}
