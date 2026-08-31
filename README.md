# Bloqueio Transparente

Aplicativo nativo para Windows 10 e 11 que bloqueia teclado e mouse sem ocultar ou congelar o conteúdo dos monitores.

Feito por Gabriel Paz.

## Versão atual

**6.6** (`0.6.6`)

## Recursos

- Bloqueio transparente em múltiplos monitores
- Senha personalizada, inclusive vazia
- Windows Hello como mecanismo exclusivo de desbloqueio quando ativado
- Bloqueio automático após 1, 5, 10, 15, 30 ou 60 minutos de inatividade
- Atalho global configurável
- Substituição opcional de `Win + L` com recuperação pelo serviço
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

- Windows 10 de 32 bits: instalador x86
- Windows 10 ou 11 de 64 bits: instalador x64
- Permissão de administrador para instalar o serviço

## Instalação

Baixe o instalador na seção **Releases** e abra-o.

Na primeira execução:

1. Defina a senha.
2. Confirme a senha.
3. Clique em **Instalar e iniciar**.

Após a instalação, o aplicativo será iniciado automaticamente com o Windows. Ao abrir o instalador novamente, será possível abrir as configurações, atualizar ou desinstalar a versão existente.

Para atualizar uma instalação anterior, abra o instalador 6.6 da mesma arquitetura e escolha **Atualizar**.

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

Com o Windows Hello desativado, a primeira tecla imprimível abre o campo de senha e já é incluída na entrada. A tecla `Escape` oculta o campo sem desbloquear.

Com o Windows Hello ativado, mover o mouse ou pressionar uma tecla abre a confirmação nativa do Windows. O teclado e o leitor biométrico ficam livres enquanto essa confirmação estiver aberta. A senha do app não é aceita para desbloquear, mas continua protegendo as configurações.

Se a janela de senha ou a confirmação do Windows Hello ficar 15 segundos sem atividade, ela será fechada sem desbloquear a tela. Uma nova entrada abre a confirmação novamente.

## Alterações da versão 6.6

- O evento de liberação da tecla Windows chega ao sistema após um bloqueio iniciado por `Win + L`, evitando que o modificador permaneça pressionado.
- O relógio e a data do widget ficam brancos em 0% de transparência.

## Alterações da versão 6.5

- A ativação do Windows Hello é executada fora da thread da interface, evitando falha de inicialização e travamento do botão.
- O botão fica desabilitado enquanto a confirmação do Windows Hello estiver aberta.

## Alterações da versão 6.4

- `Win + L` usa os eventos do hook para rastrear as teclas Windows e acionar o bloqueio transparente de forma consistente.

## Alterações da versão 6.3

- O controle do widget agora representa transparência: 0% mantém as letras brancas e 100% deixa o widget invisível.
- O texto do controle foi alterado de **Opacidade** para **Transparência**.
- Novas configurações usam 15% de transparência e ocultam a barra de tarefas por padrão.
- A tela de aparência mostra uma prévia do logotipo de desbloqueio selecionado.
- A ativação do Windows Hello funciona quando a janela de configurações já inicializou o componente de interface do Windows.

## Alterações da versão 6.2

- Fechamento automático da janela de senha após 15 segundos sem atividade.
- Cancelamento da confirmação do Windows Hello após o mesmo período.
- A tela permanece bloqueada e volta a exibir somente as coberturas transparentes.

## Alterações da versão 6.1

- Opção **Encerrar aplicativo** no menu do ícone da bandeja.
- O encerramento para o serviço, o agente e as demais instâncias do executável instalado.
- A configuração de `Win + L` é restaurada antes do encerramento.
- O pedido de encerramento só é aceito quando enviado pelo agente iniciado pelo serviço.

## Alterações da versão 6.0

- Novo instalador x86 para Windows 10 de 32 bits.
- Instaladores separados e identificados para x86 e x64.
- Comando `--app-architecture` para confirmar a arquitetura do executável.

## Alterações da versão 5.9

- Slider para regular a opacidade do widget entre 0% e 100%.
- Remoção do fundo e do contorno retangular do relógio.
- Escurecimento da tela em 40% por padrão para novos usuários.
- Configurações existentes mantêm seus valores atuais.

## Alterações da versão 5.8

- O hook permanente de `Win + L` continua instalado durante o Windows Hello.
- `Win + L` volta a funcionar após desbloquear e pode ser usado repetidamente.

## Alterações da versão 5.7

- Opção autenticada para usar `Win + L` com o bloqueio transparente.
- O hook é confirmado antes de desativar o bloqueio nativo do Windows.
- O valor anterior da política de bloqueio é preservado e restaurado.
- O serviço restaura `Win + L` antes de reiniciar o agente e quando é encerrado.
- Falhas repetidas continuam acionando o bloqueio normal do Windows.

## Alterações da versão 5.6

- Os hooks de teclado e mouse liberam os eventos enquanto o Windows Hello está aberto.
- A thread de verificação do Windows Hello usa o modelo adequado para aguardar a resposta assíncrona.
- O widget ganhou os tamanhos muito pequeno e muito grande.

## Alterações da versão 5.5

- A janela transparente não retoma o foco enquanto o Windows Hello está aberto.
- O teclado permanece disponível para digitar o PIN do Windows Hello.
- A confirmação biométrica pode concluir sem disputa de foco com a janela de bloqueio.

## Alterações da versão 5.4

- A confirmação do Windows Hello não interrompe mais os sinais de vida do agente.
- O agente volta a iniciar após o desbloqueio do Windows sem desativar a proteção configurada.
- A numeração dos instaladores voltou a avançar em 0.1 por versão.

- Opção de bloqueio automático por tempo sem usar teclado ou mouse.
- Tempo configurável em 1, 5, 10, 15, 30 ou 60 minutos, desativado por padrão.
- Windows Hello implementado como único mecanismo de desbloqueio quando ativado.
- Ativação condicionada à disponibilidade e a uma confirmação válida do Windows Hello.
- Cancelamento ou falha do Windows Hello mantém a tela bloqueada e permite uma nova tentativa.
- Substituição de `Win + L` suspensa. O atalho mantém o bloqueio normal do Windows.
- A opção antiga de substituição de `Win + L` é desativada durante a atualização.
- Widget de data e hora redesenhado com maior destaque para a hora.
- Data exibida em formato compacto em português.
- Posição configurável e cinco tamanhos de widget: muito pequeno, pequeno, médio, grande e muito grande.

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
- Quando a substituição de `Win + L` está ativa, as formas normais de bloquear o Windows ficam temporariamente desativadas.
- O Windows Hello precisa estar configurado para o usuário no Windows.
- Um administrador local pode encerrar ou modificar o aplicativo.
- RDP, toque, caneta e assinatura digital não fazem parte desta versão.
- O bloqueio transparente não substitui a segurança da tela protegida do Windows.

## Licença

MIT
