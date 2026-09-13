# Instalador e assinatura

## Arquivos e manutenção

`BloqueioTransparente.exe` é o aplicativo/serviço. `BloqueioTransparente-Setup-0.7.0-x64.exe` é um executável distinto, com o aplicativo incorporado e manifesto `requireAdministrator`. O instalador reaproveita as telas de configuração e manutenção, mas não se copia como se fosse o aplicativo.

O aplicativo é gravado na pasta Program Files informada pelo Windows. O instalador completo fica no subdiretório `Installer`, com versão no nome. A entrada `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\BloqueioTransparente` registra `DisplayVersion`, `ModifyPath` e `UninstallString`. O instalador guardado permite reparar o executável principal mesmo quando ele foi apagado. As pastas do Windows são obtidas por Known Folders, incluindo área de trabalho redirecionada.

Atualização e reparação validam a configuração existente antes de parar o serviço e não gravam nela. Uma versão mais antiga não substitui uma mais recente. Cada operação adquire um mutex global, para impedir duas instalações simultâneas. O serviço é parado antes de substituir arquivos. Cópias temporárias permitem restaurar o executável, o instalador guardado e os atalhos se alguma etapa falhar; a definição e o estado anterior do serviço também são restaurados quando possível. Se o Windows negar a restauração, o erro informa a falha e o caminho da cópia anterior preservada. Isso cobre erros tratados, não oferece garantia contra desligamento abrupto no meio da instalação.

O encerramento da tela de configuração fica suspenso enquanto a instalação está em andamento. Atualização e reparação são executadas em um processo separado do aplicativo, que não é encerrado junto com o serviço. A desinstalação mantém o arquivo de configuração protegido.

## Assinatura

Um instalador não elimina as verificações dos arquivos que instala. A Microsoft recomenda assinar também os binários internos para [compatibilidade com Smart App Control](https://learn.microsoft.com/en-us/windows/apps/develop/smart-app-control/code-signing-for-smart-app-control). O [SmartScreen considera reputação](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation); uma assinatura válida não promete ausência de todos os avisos ou aceitação por qualquer política corporativa.

O processo de distribuição exige um certificado de assinatura de código confiável com acesso à chave privada, ou um serviço de assinatura Authenticode configurado. Não cria certificados próprios, não instala raízes de confiança e não muda políticas do Windows. Não há certificado emitido incluído neste projeto.

Ordem obrigatória em `scripts/package.ps1`:

1. Compilar o aplicativo e conferir sua versão.
2. Assinar o aplicativo com SHA-256 e carimbo de tempo; verificar com SignTool `/pa /all` e Authenticode.
3. Incorporar esses bytes ao instalador e compilá-lo.
4. Assinar e verificar o instalador.
5. Conferir arquitetura, hashes, manifesto de administrador e presença exata do aplicativo no instalador.
6. Promover o diretório temporário para `dist`. Uma falha de compilação, assinatura ou verificação não promove arquivos parciais. Pacotes assinados de uma versão existente não são sobrescritos.

A validação do certificado usa a confiança configurada no Windows que assina. O certificado escolhido para distribuição pública deve encadear a uma autoridade reconhecida publicamente, e não apenas a uma raiz adicionada localmente. A documentação da Microsoft apresenta [opções de assinatura](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options).

## Compilar

É necessário PowerShell 7, Rust, Windows SDK/SignTool para assinatura e Python 3 para inspeção do pacote. O backend `Native` usa a cadeia de compilação MSVC. O backend `Wsl` usa Rust, cargo-xwin, clang, lld, llvm-rc e o alvo Rust correspondente no WSL. As dependências permanecem fixadas em Cargo.lock. O alvo x64 é `x86_64-pc-windows-msvc`; x86 é `i686-pc-windows-msvc`.

Certificado já instalado, sem passar senhas pela linha de comando:

```powershell
pwsh -File scripts/package.ps1 -BuildBackend Wsl -CertificateThumbprint "THUMBPRINT_DO_CERTIFICADO"
```

Use `-MachineCertificateStore` se o certificado estiver em `LocalMachine\My`. A chave pode ser gerenciada por token ou provedor compatível com SignTool; a autenticação desse provedor é externa ao script.

Serviço compatível com a integração Dlib do SignTool, usando os arquivos fornecidos pelo serviço:

```powershell
pwsh -File scripts/package.ps1 -SigningDlib "C:\Signing\Azure.CodeSigning.Dlib.dll" -SigningMetadata "C:\Signing\metadata.json"
```

Endpoint, conta, perfil e autenticação devem ser configurados no serviço. A identidade, elegibilidade e emissão de certificado não são automatizadas pelo projeto.

Para desenvolvimento, sem assinatura e sem indicação de que está pronto para distribuição:

```powershell
pwsh -File scripts/package.ps1 -BuildBackend Wsl -UnsignedDevelopment
```

O resultado fica em `dist/0.7.0/x64/unsigned-development`. O Windows pode bloquear esse pacote e o aplicativo. Para x86, acrescente `-Architecture x86` e instale o alvo correspondente no Rust do backend escolhido. Não distribua o aplicativo intermediário separadamente; o arquivo destinado ao usuário é o instalador.

## Verificação

```powershell
cargo fmt --all -- --check
pwsh -File scripts/test-signing-gate.ps1
py -3 scripts/verify-package.py dist/0.7.0/x64/unsigned-development
```

No WSL, `scripts/build-windows.sh test` executa os testes portáveis, excluindo dois testes de separadores de caminhos próprios do Windows. `check` e `check-installer` executam Clippy para Windows; a última opção recebe o caminho WSL do aplicativo que será incorporado. A inspeção Python não executa os binários nem substitui a verificação criptográfica por SignTool.

`scripts/diagnose-windows.ps1` consulta assinaturas, estado do Smart App Control e eventos 3077/3089/3033 de CodeIntegrity. O comando é somente leitura e permite salvar o resultado em JSON com `-OutputPath`.

## Aceitação em Windows

Os cenários abaixo precisam ser executados em uma máquina de teste com o pacote assinado. Não foram comprovados por testes de arquivos no WSL.

- Instalação nova pela interface: senha e atalhos, serviço iniciado, aplicativo na bandeja, entrada em Aplicativos instalados.
- Atualização de 6.7/6.8/6.9, com senha, Windows Hello e preferências preservados.
- Reparo na mesma versão; depois, reparo com o executável principal ausente.
- Tentativa de downgrade; deve falhar antes de parar o serviço.
- Falha de início do serviço e restauração da versão anterior; conferir eventuais erros do Windows.
- Desinstalação pela entrada do Windows, incluindo arquivos em uso, e reinstalação preservando configurações.
- Bloqueio/desbloqueio transparente e nativo, com restauração da barra de tarefas.

Há um teste automatizado para atualizar e reparar uma instalação existente, inclusive executável ausente. Requer PowerShell elevado, assinatura válida e identificação explícita de máquina de teste. Não executa desinstalação:

```powershell
pwsh -File scripts/test-installed-package.ps1 -Installer "C:\Pacotes\BloqueioTransparente-Setup-0.7.0-x64.exe" -ExpectedVersion 0.7.0 -Mode Update -TestMachine
pwsh -File scripts/test-installed-package.ps1 -Installer "C:\Pacotes\BloqueioTransparente-Setup-0.7.0-x64.exe" -ExpectedVersion 0.7.0 -Mode RepairMissingExecutable -TestMachine
```

O teste compara o hash da configuração antes e depois, a versão, as assinaturas, o registro de instalação e o estado do serviço. A confirmação visual de interface, hooks e barra de tarefas continua necessária.
