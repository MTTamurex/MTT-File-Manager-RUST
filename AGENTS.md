## ⛔ REGRA ZERO - VERIFICAÇÃO OBRIGATÓRIA ANTES DE QUALQUER AÇÃO

```
╔══════════════════════════════════════════════════════════════════════════════╗
║  ANTES DE EXECUTAR QUALQUER COMANDO, EDITAR QUALQUER ARQUIVO OU PROPOR      ║
║  QUALQUER SOLUÇÃO, O AGENTE DEVE COMPLETAR ESTE CHECKLIST:                  ║
╠══════════════════════════════════════════════════════════════════════════════╣
║                                                                              ║
║  □ 1. ENTENDI O PEDIDO?                                                     ║
║       - O que EXATAMENTE o usuário pediu?                                   ║
║       - Estou assumindo algo que não foi dito? → SE SIM, PERGUNTE          ║
║       - Há ambiguidade no pedido? → SE SIM, PERGUNTE                       ║
║                                                                              ║
║  □ 2. LI O CÓDIGO RELEVANTE?                                                ║
║       - Li os arquivos que vou modificar? → SE NÃO, LEIA PRIMEIRO          ║
║       - Entendi como funciona atualmente? → SE NÃO, INVESTIGUE             ║
║       - Sei quais são as dependências? → SE NÃO, MAPEIE                    ║
║                                                                              ║
║  □ 3. ESTOU NO ESCOPO?                                                      ║
║       - Isso foi explicitamente solicitado? → SE NÃO, NÃO FAÇA             ║
║       - Estou adicionando algo "extra"? → SE SIM, PARE E PERGUNTE          ║
║       - Isso pode quebrar algo existente? → SE SIM, ALERTE O USUÁRIO       ║
║                                                                           ║  ║                                                                            ║
║  □ 4. COMANDO DESTRUTIVO?                                                  ║
║       - É um comando que deleta/limpa dados? (rm, clean, reset, etc)       ║
║       - → SE SIM: NUNCA execute sem permissão EXPLÍCITA do usuário         ║
║       - → Alerte sobre consequências ANTES de executar                    ║  
║                                                                            ║
║  □ 5. ESSA ALTERAÇÃO IRÁ "QUEBRAR" A FUNCIONALIDADE DE OUTRAS FUNÇÕES?     ║
║                                                                            ║
║       - → SE SIM: NUNCA execute sem permissão EXPLÍCITA do usuário         ║
║       - → Alerte sobre consequências ANTES de executar                     ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  ⚠️  SE QUALQUER ITEM ACIMA FALHAR → PARE E PERGUNTE AO USUÁRIO            ║
║  ⚠️  ESTAS REGRAS TÊM PRIORIDADE SOBRE QUALQUER "INSTINTO" OU OTIMIZAÇÃO   ║
║  ⚠️  VELOCIDADE NÃO JUSTIFICA PULAR VERIFICAÇÕES                           ║
╚══════════════════════════════════════════════════════════════════════════════╝
```
## Princípios Fundamentais

### 1. Precisão Técnica Acima de Validação
- Forneça informações técnicas objetivas
- Discorde quando necessário
- Investigue para encontrar a verdade antes de confirmar suposições

### 2. Comunicação Direta e Concisa
- Respostas curtas e focadas
- Evite superlativos desnecessários
- Use markdown para formatação

### 3. Minimalismo - Evite Over-Engineering
- Apenas mudanças diretamente solicitadas
- Soluções simples e focadas
- Não adicione features não solicitadas
- Não refatore código além do necessário

### 4. Perguntas Antes de Suposições
- Quando requisitos são ambíguos, PERGUNTE
- Quando há múltiplas abordagens, APRESENTE AS OPÇÕES
- Não assuma - valide entendimento

### 5. Limpeza de Código
- Se algo não é usado, DELETE completamente
- Não mantenha código morto "por precaução"
- Siga a Regra dos Três: três linhas similares são melhores que uma abstração prematura
  
### 6. Proibido “God Files” / Monólitos (modularização obrigatória)
- Nunca crie ou concentre grandes mudanças em um único arquivo “monolito” (“god file”). 
- Se a implementação começar a ficar grande (400~500 linhas), divida obrigatoriamente em módulos/coisas menores, com responsabilidades claras e limites bem definidos.
- Tamanho e complexidade como gatilho: ao perceber que um arquivo está crescendo demais (muitas responsabilidades, muitas funções não relacionadas, muitos tipos/structs/classes), pare e proponha a divisão antes de continuar.