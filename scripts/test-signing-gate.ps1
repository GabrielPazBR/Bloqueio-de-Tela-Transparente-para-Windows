#requires -Version 7.0
$ErrorActionPreference = 'Stop'
$package = Join-Path $PSScriptRoot 'package.ps1'
$cases = @(
    @{ Arguments = @(); Expected = 'Assinatura obrigatória' },
    @{ Arguments = @('-UnsignedDevelopment', '-CertificateThumbprint', '0000'); Expected = 'Não combine' },
    @{ Arguments = @('-SigningDlib', 'missing.dll'); Expected = 'Assinatura obrigatória' }
)
foreach ($case in $cases) {
    $result = & (Join-Path $PSHOME 'pwsh.exe') -NoProfile -File $package @($case.Arguments) 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0 -or -not $result.Contains($case.Expected)) {
        throw "A barreira de assinatura não rejeitou os parâmetros esperados: $result"
    }
}
foreach ($script in Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1') {
    $tokens = $null
    $errors = $null
    $null = [System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw "Erro de sintaxe em $($script.Name): $errors" }
}
'3 cenários de assinatura rejeitados corretamente; scripts PowerShell sem erros de sintaxe.'
