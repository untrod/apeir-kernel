param([switch]$Release)
. "$PSScriptRoot\common.ps1"
Set-Location -LiteralPath $script:RepositoryRoot
$arguments = @("build", "--workspace", "--locked")
if ($Release) { $arguments += "--release" }
Invoke-NousCargo $arguments
