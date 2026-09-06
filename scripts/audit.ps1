. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot
Invoke-NousCargo @("fmt", "--all", "--", "--check")
Invoke-NousCargo @("clippy", "--workspace", "--all-targets", "--", "-D", "warnings")
Invoke-NousPython @("-m", "compileall", "-q", "sdk/python/src", "sdk/python/tests", "tests/e2e", "scripts")
Invoke-NousPython @("-m", "ruff", "check", "sdk/python", "tests", "scripts")
Invoke-NousPython @("scripts/architecture_check.py")
Invoke-NousPython @("scripts/release_audit.py", "--output", ".local/audit")
if (Test-Path -LiteralPath (Join-Path $script:RepositoryRoot ".git")) {
    git diff --check
    if ($LASTEXITCODE -ne 0) { throw "git diff check failed" }
}
