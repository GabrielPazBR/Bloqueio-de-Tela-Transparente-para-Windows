#requires -Version 7.0
[CmdletBinding()]
param([string[]]$Executable = @(), [string]$OutputPath)
$ErrorActionPreference = 'Stop'
$policy = Get-ItemProperty -LiteralPath 'HKLM:/SYSTEM/CurrentControlSet/Control/CI/Policy' -ErrorAction SilentlyContinue
$events = @(Get-WinEvent -FilterHashtable @{
    LogName = 'Microsoft-Windows-CodeIntegrity/Operational'; Id = 3077, 3089, 3033
} -MaxEvents 50 -ErrorAction SilentlyContinue | ForEach-Object {
    [ordered]@{ time = $_.TimeCreated; id = $_.Id; message = $_.Message; xml = $_.ToXml() }
})
$files = @($Executable | ForEach-Object {
    $signature = Get-AuthenticodeSignature -LiteralPath $_
    [ordered]@{
        path = $_; version = [Diagnostics.FileVersionInfo]::GetVersionInfo($_).FileVersion
        signatureStatus = [string]$signature.Status
        signer = $signature.SignerCertificate.Subject
        sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash
    }
})
$report = [ordered]@{
    time = (Get-Date).ToString('o')
    smartAppControlState = $policy.VerifiedAndReputablePolicyState
    files = $files; codeIntegrityEvents = $events
    note = 'Somente leitura. Não altera políticas nem instala certificados.'
} | ConvertTo-Json -Depth 8
if ($OutputPath) { $report | Set-Content -LiteralPath $OutputPath -Encoding utf8 } else { $report }
