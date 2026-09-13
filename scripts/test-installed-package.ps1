#requires -Version 7.0
#requires -RunAsAdministrator
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][string]$ExpectedVersion,
    [ValidateSet('Update', 'Repair', 'RepairMissingExecutable')][string]$Mode = 'Repair',
    [switch]$TestMachine
)
$ErrorActionPreference = 'Stop'
if (-not $TestMachine) { throw 'Execute somente em uma máquina de teste, com -TestMachine.' }
$Installer = (Resolve-Path -LiteralPath $Installer).Path
if ((Get-AuthenticodeSignature -LiteralPath $Installer).Status -ne 'Valid') {
    throw 'O teste exige um instalador com assinatura válida.'
}
$configuration = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'Bloqueio Transparente/config.json'
$before = (Get-FileHash -LiteralPath $configuration -Algorithm SHA256).Hash
$service = Get-CimInstance Win32_Service -Filter "Name='BloqueioTransparente'"
if (-not $service -or $service.PathName -notmatch '^"(.+)" --service$') { throw 'Instalação anterior não encontrada.' }
$application = $Matches[1]
if ([IO.Path]::GetFileName($application) -ne 'BloqueioTransparente.exe') { throw 'O serviço aponta para um aplicativo inesperado.' }
$backup = "$application.acceptance-backup"
if (Test-Path -LiteralPath $backup) { throw 'Já existe uma cópia de teste anterior.' }
try {
    if ($Mode -eq 'RepairMissingExecutable') {
        Stop-Service BloqueioTransparente
        (Get-Service BloqueioTransparente).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30))
        Move-Item -LiteralPath $application -Destination $backup
    }
    $argument = if ($Mode -eq 'Update') { '--update-quiet' } else { '--repair-quiet' }
    $process = Start-Process -FilePath $Installer -ArgumentList $argument -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit(120000)) { throw 'O instalador não concluiu em dois minutos; verifique a máquina de teste.' }
    if ($process.ExitCode -ne 0) { throw "O instalador retornou $($process.ExitCode)." }
    if ((Get-FileHash -LiteralPath $configuration -Algorithm SHA256).Hash -ne $before) { throw 'As configurações foram alteradas.' }
    if ([Diagnostics.FileVersionInfo]::GetVersionInfo($application).FileVersion -ne $ExpectedVersion) { throw 'Versão instalada incorreta.' }
    if ((Get-AuthenticodeSignature -LiteralPath $application).Status -ne 'Valid') { throw 'Assinatura do aplicativo inválida.' }
    if ((Get-Service BloqueioTransparente).Status -ne 'Running') { throw 'O serviço não está em execução.' }
    $registration = Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Windows/CurrentVersion/Uninstall/BloqueioTransparente'
    if ($registration.DisplayVersion -ne $ExpectedVersion) { throw 'Registro de instalação incorreto.' }
    if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup }
    "PASS: $Mode; configurações idênticas; versão $ExpectedVersion; assinaturas válidas; serviço iniciado."
} catch {
    if ((Test-Path -LiteralPath $backup) -and -not (Test-Path -LiteralPath $application)) {
        Move-Item -LiteralPath $backup -Destination $application
        Start-Service BloqueioTransparente
    }
    throw
}
