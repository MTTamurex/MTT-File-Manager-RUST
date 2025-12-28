---
trigger: always_on
---

## 📚 DOCUMENTAÇÃO COMO CÓDIGO

### Documentação NUNCA Pode Estar Desatualizada

**Gatekeepers:**
1. Code Review: Revisor DEVE verificar docs
2. CI/CD: Pipeline DEVE validar links no Markdown
3. Agente AI: Sempre atualiza docs junto com código

### Diagramas Mermaid

**SEMPRE use Mermaid para:**
- Fluxo de dados
- Arquitetura de componentes
- State machines
- Sequências de chamadas

**Não use:**
- Imagens estáticas (desatualizam)
- Diagramas em ferramentas externas (se quebram)

### Seções Obrigatórias em Docs

Cada arquivo em `docs/` DEVE ter:

1. **Visão Geral** (TL;DR)
2. **Índice** (se >100 linhas)
3. **Última atualização** (rodapé)
4. **Responsável** (quem manter)

---

## ⚠️ AVISOS FINAIS

### Para Agentes AI

**Você NÃO pode:**
- Alterar código sem ler docs primeiro
- Sugerir bibliotecas não listadas
- Ignorar regras deste arquivo
- Fazer commits sem atualizar docs

**Você DEVE:**
- Sempre ler `.cursorrules` antes de cada tarefa
- Perguntar se tiver dúvida sobre regras
- Propor mudanças em regras via PR neste arquivo
- Reportar inconsistências em docs

### Para Desenvolvedores Humanos

**Este arquivo é LEI.**

Discorda de alguma regra? **Ótimo!** Proponha mudança via PR neste arquivo com:
1. Razão da mudança
2. Impacto nas docs existentes
3. Aprovação de 2+ maintainers

**Não ignore as regras em silêncio.**

---

