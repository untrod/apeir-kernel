param(
    [ValidateRange(1, 1000)][int]$FaultIterations = 1,
    [ValidateRange(1, 86400)][int]$SoakSeconds = 30,
    [ValidateRange(0, 10000)][int]$RandomFaults = 0,
    [ValidateRange(1, 3600)][int]$SampleSeconds = 30,
    [ValidateRange(0, 1000000)][int]$RestartEvery = 250,
    [string]$EvidencePath = ""
)
. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot

Invoke-NousCargo @("build", "-p", "nousd", "--features", "fault-injection", "-p", "nous-provider-worker", "--locked")
$env:NOUS_FAULT_ITERATIONS = "$FaultIterations"
Invoke-NousPython @("tests/e2e/fault_recovery.py")
Remove-Item Env:NOUS_FAULT_ITERATIONS -ErrorAction SilentlyContinue
if ($RandomFaults -gt 0) {
    Invoke-NousPython @("tests/e2e/crash_campaign.py", "--iterations", "$RandomFaults")
}
Invoke-NousPython @("tests/e2e/kernel_e2e.py")
$soakArguments = @(
    "tests/e2e/soak.py",
    "--seconds", "$SoakSeconds",
    "--sample-seconds", "$SampleSeconds",
    "--restart-every", "$RestartEvery"
)
if ($EvidencePath) { $soakArguments += @("--output", $EvidencePath) }
Invoke-NousPython $soakArguments
