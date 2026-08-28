# Bloqueio Transparente

Aplicativo nativo para Windows 11 x64 que bloqueia teclado e mouse sem ocultar ou congelar o conteúdo dos monitores.

Feito por Gabriel Paz.

## Versão atual

**5.0b** (`0.5.0-beta`)

[Baixar o instalador para Windows 11 x64](https://github.com/GabrielPazBR/Bloqueio-de-Tela-Transparente-para-Windows/releases/download/v0.5.0-beta/BloqueioTransparente-Setup-0.5.0b.exe)

## Recursos

- Bloqueio transparente em múltiplos monitores
- Senha personalizada, inclusive vazia
- Atalho global configurável
- Ícone na bandeja do sistema
- Escurecimento ajustável da tela
- Mensagem personalizada na tela de desbloqueio
- Widget configurável de data e hora ou imagem
- Relógio com hora em destaque e data compacta
- Inicialização automática com o Windows
- Serviço de recuperação do agente
- Limitação progressiva após tentativas incorretas
- Instalação, atualização e desinstalação pelo mesmo executável

## Requisitos

- Windows 11 x64
- Permissão de administrador para instalar o serviço

## Instalação

Baixe o instalador na seção **Releases** e abra-o.

Na primeira execução:

1. Defina a senha.
2. Confirme a senha.
3. Clique em **Instalar e iniciar**.

Após a instalação, o aplicativo será iniciado automaticamente com o Windows. Ao abrir o instalador novamente, será possível abrir as configurações, atualizar ou desinstalar a versão existente.

Para atualizar uma instalação anterior, abra o instalador 5.0b e escolha **Atualizar**.

## Uso

O atalho padrão para bloquear a tela é:

```text
Ctrl + Shift + L
```

Também é possível bloquear pela bandeja do sistema ou pelo terminal:

```powershell
BloqueioTransparente.exe lock
BloqueioTransparente.exe settings
BloqueioTransparente.exe status
BloqueioTransparente.exe uninstall
```

Durante o bloqueio, a primeira tecla imprimível abre o campo de senha e já é incluída na entrada. A tecla `Escape` oculta o campo sem desbloquear.

## Alterações da versão 5.0b

- Integração com Windows Hello suspensa.
- Substituição de `Win + L` suspensa. O atalho mantém o bloqueio normal do Windows.
- Opções antigas dessas integrações são desativadas durante a atualização.
- Widget de data e hora redesenhado com maior destaque para a hora.
- Data exibida em formato compacto em português.
- Posição e tamanho configuráveis do widget preservados.

## Testes

```powershell
cargo test --release
cargo clippy --release --all-targets -- -D warnings
cargo fmt --check
cargo build --release
```

Os testes comuns não instalam hooks globais, não criam o serviço e não bloqueiam a sessão.

## Limitações

- `Ctrl + Alt + Del` continua sendo controlado pelo Windows.
- `Win + L` mantém o bloqueio normal do Windows nesta versão.
- Windows Hello não está disponível nesta versão.
- Um administrador local pode encerrar ou modificar o aplicativo.
- RDP, toque, caneta e assinatura digital não fazem parte desta versão.
- O bloqueio transparente não substitui a segurança da tela protegida do Windows.

## Licença

MIT
