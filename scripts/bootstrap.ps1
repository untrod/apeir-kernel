. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot

foreach ($tool in @("git", "cargo", "rustc", "python")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Required tool is unavailable: $tool"
    }
}
Invoke-NousCargo @("--version")
Invoke-NousPython @("--version")
Invoke-NousPython @("-m", "ruff", "--version")
git --version
Write-Output "APEIR Kernel build environment is ready."
