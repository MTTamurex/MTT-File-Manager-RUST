# Prova de conceito: transferência pelo Shell

Este exemplo testa se o `IDropTarget` nativo da pasta de destino aciona a
janela de escolha do TeraCopy 4.0.5. O exemplo permanece isolado dos workers do
aplicativo. A integração de copiar/mover no MTT está descrita em
`docs/shell_transfer_integration.md`, a partir da raiz do repositório.

## Segurança e limites

- Usa somente cópia. Não solicita mover, excluir ou alterar o clipboard.
- Não muda configurações do Windows ou do TeraCopy.
- Não executa a CLI do TeraCopy e não repete a operação por outro mecanismo.
- Exige caminhos absolutos existentes e a confirmação `COPY` no console.
- **É uma cópia real:** use arquivos e pastas descartáveis, com destino vazio.
  O handler pode oferecer substituir arquivos; cancele qualquer substituição
  inesperada. O exemplo não impõe uma política de conflitos ao handler.
- Não aceita copiar para dentro da própria origem nem para sua pasta atual.
- Não testa colagem nativa, recorte, movimentação ou arraste externo do egui.

## Executar no PowerShell

Na raiz do repositório, substitua os caminhos genéricos por dados de teste:

```powershell
cargo run --example shell_transfer_probe -- "D:\Destination" "C:\Source\file.txt"
```

Para seleção múltipla, acrescente outras origens ao final. As origens podem
pertencer a pastas diferentes. O primeiro argumento é sempre o destino.

O programa confirma os argumentos, inicializa OLE em STA, cria um `IDataObject`
do Shell e obtém o `IDropTarget` da pasta. Chama `DragEnter` com botão esquerdo
e Ctrl, seguido de `Drop` com Ctrl, permitindo **somente COPY**.

Mantenha a janela da prova aberta até o fim da transferência: ela preserva os
objetos COM e bombeia mensagens. Depois, feche-a para sair. Fechar essa janela
**não equivale a cancelar a transferência**; use a interface do handler para
cancelar. O retorno de `Drop` não é anunciado como conclusão da cópia.

## Verificação manual

Use destinos de teste separados para cada execução, sem apagar dados existentes.

1. Com a integração e a confirmação do TeraCopy ativadas, execute o exemplo e
   informe se aparece a mesma janela de escolha vista no Explorer.
2. Escolha TeraCopy; confirme o conteúdo no destino e a preservação da origem.
3. Em outra execução, escolha o método nativo, se disponível, e confira o resultado.
4. Em outra execução, cancele na janela de escolha. O exemplo não deve iniciar
   outra cópia como alternativa. Confira o destino.
5. Repita com seleção múltipla e com o TeraCopy já aberto.
6. Se desejar comparar a integração desativada, altere essa opção manualmente
   no TeraCopy e restaure depois sua preferência original.

Para relatar o resultado, informe as etapas do console, o método escolhido e
o comportamento observado. Não é necessário compartilhar caminhos ou nomes reais.

Se não aparecer a janela esperada, isso **não prova impossibilidade de integração**:
o comando nativo de colagem e possíveis restrições ao processo host continuam
pendentes de investigação.

## Observar notificações posteriores ao retorno de Drop

O teste manual confirmou que a janela de escolha, o método nativo e o cancelamento
funcionam, mas `Drop` pode retornar antes do fim da cópia pelo TeraCopy. Portanto,
o retorno não pode disparar a limpeza de recorte ou uma notificação de sucesso.

Para investigar se existe feedback posterior, use um destino vazio e dados
descartáveis, preferencialmente uma cópia demorada o suficiente para distinguir
o início e o fim:

```powershell
cargo run --example shell_transfer_probe -- --observe "D:\Destination" "C:\Source\file.txt"
```

O modo `--observe` envolve o objeto de dados com um wrapper de diagnóstico que
oferece `IDataObjectAsyncCapability` com modo assíncrono ativado e registra:

- `Started` / `Ended`: chamadas de `StartOperation` e `EndOperation`.
- `Feedback`: `Performed DropEffect`, `Logical Performed DropEffect` e
  `Paste Succeeded`, com o valor informado e se o objeto interno aceitou o dado.
- `DropReturned`: ponto em que a chamada ao destino já retornou.
- Um resumo das quantidades recebidas, ao fechar a janela da prova.

As linhas `[Observe +tempo]` permitem comparar a ordem dos sinais com o fim
visível da transferência. **Mantenha a janela aberta até a cópia terminar**,
aguarde alguns segundos e só então feche para obter o resumo. Repita escolhendo
o método nativo e, em outra execução, cancelando na janela de escolha.

Compartilhe as linhas de diagnóstico e indique se `Ended`/`Feedback` aparecem
antes ou depois do fim real. Ausência de feedback também é um resultado útil.
Nenhum sinal é tratado automaticamente como confirmação de sucesso.

Esse modo é experimental: o wrapper expõe as duas interfaces de transferência,
não todas as interfaces extras do objeto original, e habilita a negociação
assíncrona. Isso pode mudar o comportamento ou o tempo de retorno do Shell.
O modo **sem `--observe` permanece com o objeto original e modo assíncrono
desativado**, preservando o controle já validado. O teste continua apenas de
cópia e não altera o clipboard.

## Verificações sem transferência

```powershell
cargo test --example shell_transfer_probe
cargo run --example shell_transfer_probe -- --help
```

Os testes cobrem os argumentos, a comparação de ancestralidade, a conversão de
prefixos verbatim DOS/UNC para nomes aceitos pelo Shell e a resolução somente
leitura do executável de teste. Não criam arquivos, copiam dados ou invocam o
destino de transferência. Os testes do observador usam um objeto simulado para
verificar o protocolo assíncrono, a leitura dos efeitos e a propriedade dos
blocos de memória. Não validam o comportamento do TeraCopy; essa parte exige a
verificação interativa acima.
