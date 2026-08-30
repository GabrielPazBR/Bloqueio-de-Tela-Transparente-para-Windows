# Alternativas para usar Win+L no bloqueio transparente

Data da pesquisa: 2026-08-29

## Conclusão

O Windows não oferece uma API Win32 comum e suportada para substituir diretamente a ação de `Win+L`. Para Windows Home e Pro, a alternativa viável é uma integração opcional que combina a política `DisableLockWorkstation` com um hook `WH_KEYBOARD_LL` ativo enquanto o agente estiver funcionando. Essa solução desativa a função nativa de bloqueio para o usuário e exige recuperação automática após falha.

A opção mais compatível e segura continua sendo manter `Win+L` com o Windows e usar um atalho diferente no aplicativo.

## Comparação

| Alternativa | Resultado | Limitação principal |
| --- | --- | --- |
| `RegisterHotKey` | Não serve para `Win+L` | Atalhos com a tecla Windows são reservados ao sistema |
| `WH_KEYBOARD_LL` isolado | Pode observar e consumir eventos comuns | Não substitui com garantia a ação nativa e o hook pode ser removido após timeout |
| `DisableLockWorkstation` + `WH_KEYBOARD_LL` | Viável em edições comuns | Remove toda a função Bloquear do usuário, não apenas `Win+L` |
| Keyboard Filter | Bloqueia `Win+L` antes do sistema | Disponível somente em edições específicas e bloqueia, mas não redireciona para o aplicativo |
| Remapeamento pelo PowerToys | Não serve para `Win+L` | O próprio Keyboard Manager trata `Win+L` como combinação reservada |
| Credential Provider | Personaliza a autenticação nativa | Não intercepta `Win+L` e não mantém a área de trabalho transparente |
| `NoLockScreen` | Não resolve | Altera a tela exibida depois do bloqueio, sem impedir o bloqueio da estação |

## Proposta para protótipo opcional

1. Manter o recurso desativado por padrão e explicar que o bloqueio transparente não equivale ao bloqueio protegido do Windows.
2. Instalar e confirmar o hook global antes de alterar `DisableLockWorkstation`.
3. Manter o hook ativo também enquanto a tela estiver liberada. Hoje os hooks são instalados somente após o bloqueio transparente começar.
4. Ao detectar `Win+L`, consumir a sequência e enviar uma mensagem ao agente para iniciar o bloqueio transparente.
5. Restaurar o valor anterior da política no encerramento normal, atualização e desinstalação.
6. Fazer o serviço restaurar a política ao perder o heartbeat do agente, antes de tentar reiniciá-lo.
7. Restaurar a política antes de cada inicialização do agente para corrigir resíduos de desligamento abrupto.
8. Se o agente falhar repetidamente, manter o bloqueio nativo habilitado e suspender a substituição.

## Riscos que permanecem

- Se a política ficar ativa e o agente não estiver funcionando, `Win+L`, o menu Iniciar e a opção exibida após `Ctrl+Alt+Del` não poderão bloquear a estação.
- Um hook de baixo nível precisa responder rapidamente. O Windows pode removê-lo silenciosamente após timeout.
- O bloqueio transparente continua no desktop interativo e pode ser encerrado por um administrador.
- Políticas corporativas podem sobrescrever ou impedir a configuração.

## Recomendação

Implementar primeiro como recurso experimental e opcional, com uma recuperação controlada pelo serviço. O protótipo deve incluir testes de falha do agente, encerramento forçado, reinicialização, suspensão, troca de usuário, atualização e desinstalação. Se o requisito não exigir exatamente `Win+L`, usar `Ctrl+Shift+L` ou outro atalho configurável elimina a alteração de política e é a solução recomendada.

## Fontes primárias

- Microsoft, `RegisterHotKey`: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
- Microsoft, `LowLevelKeyboardProc`: https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc
- Microsoft, política `DisableLockWorkstation`: https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-admx-ctrlaltdel
- Microsoft, combinações predefinidas do Keyboard Filter: https://learn.microsoft.com/en-us/windows/configuration/keyboard-filter/predefined-key-combinations
- Microsoft, referência WMI do Keyboard Filter e edições compatíveis: https://learn.microsoft.com/en-us/windows/configuration/keyboard-filter/keyboardfilter-wmi-provider-reference
- Microsoft, PowerToys Keyboard Manager: https://learn.microsoft.com/pt-br/windows/powertoys/keyboard-manager
- Microsoft, Credential Providers: https://learn.microsoft.com/en-us/windows/win32/secauthn/credential-providers-in-windows
