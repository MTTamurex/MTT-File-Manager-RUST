# Relatorio de auditoria de seguranca - 2026-04-26

## Escopo

Auditoria estatica do aplicativo MTT File Manager, com foco no servico `mtt-search-service`, que pode rodar como `NT AUTHORITY\SYSTEM` em producao ou elevado em modo console. A analise cobriu:

- IPC via Named Pipes, autenticacao/autorizacao e isolamento entre app e servico.
- Instalacao/execucao do Windows Service.
- Execucao de processos, ShellExecute, shell verbs e helpers elevados.
- Leitura/escrita de arquivos, extracao de arquivos compactados e traversal.
- Carregamento de DLLs e riscos de DLL search-order hijacking.
- Deserializacao, framing de mensagens e limites de recursos.
- Persistencia do indice, ACLs e caches usados pelo servico elevado.

Nao foi executado fuzzing, exploracao dinamica nem teste multiusuario real. As severidades abaixo sao baseadas no impacto pratico em uma maquina Windows local com usuarios autenticados e malware rodando sem elevacao.

## Resumo executivo

Nao encontrei vulnerabilidade critica direta que permita execucao arbitraria como SYSTEM apenas enviando uma mensagem IPC. O servico ja possui varias protecoes importantes: `FILE_FLAG_FIRST_PIPE_INSTANCE`, `PIPE_REJECT_REMOTE_CLIENTS`, limites de payload, limites de clientes, cooldown para `WarmIndex`, HMAC em indices binarios, DACL restritiva em `C:\ProgramData\MTT-File-Manager`, endurecimento de DLL search order e autorizacao por impersonacao nos resultados de busca.

Os principais riscos confirmados estao em volta do limite de confianca do servico:

1. Instalacao manual do servico a partir de diretorio gravavel pelo usuario pode virar elevacao local de privilegio na proxima inicializacao do servico.
2. A verificacao do servidor do pipe no cliente aceita `ERROR_ACCESS_DENIED` como prova de legitimidade, mas esse erro tambem pode ocorrer contra processo de outro usuario. Isso deixa uma janela de spoofing/squatting quando o servico real nao esta rodando.
3. O pipe do servico aceita `Authenticated Users` e algumas operacoes nao fazem autorizacao equivalente ao usuario antes de revelar metadados agregados ou consumir trabalho elevado.

## Achados

### SEC-01 - Instalacao manual do servico a partir de caminho gravavel pelo usuario

**Severidade:** alta

**Modulo afetado:** [crates/mtt-search-service/src/service_control.rs](../crates/mtt-search-service/src/service_control.rs)

**Trecho relevante:**

```rust
let exe_path = std::env::current_exe().expect("Cannot get executable path");
// ...
executable_path: exe_path.clone(),
account_name: None,
account_password: None,
```

**Descricao**

`install_service()` registra no SCM exatamente o caminho retornado por `current_exe()`. O instalador normal usa `{autopf}\MTT File Manager`, que tende a herdar ACLs seguras de `Program Files`. O problema aparece no fluxo manual documentado em [README.md](../README.md) e [docs/02_build_run_debug.md](02_build_run_debug.md), onde o usuario pode executar `target\release\mtt-search-service.exe install` a partir do proprio repositorio ou outro diretorio sob o perfil do usuario.

Como o servico roda como LocalSystem, se o `ImagePath` ficar apontando para um caminho modificavel por usuario nao administrador, qualquer malware rodando nesse usuario pode substituir o binario e obter execucao como SYSTEM quando o servico for iniciado/reiniciado.

**Impacto / risco potencial**

Elevacao local de privilegio de usuario comum para SYSTEM, desde que o servico tenha sido instalado previamente de um diretorio gravavel pelo usuario.

**Cenario realista de exploracao**

1. Administrador/desenvolvedor executa `mtt-search-service.exe install` a partir de `C:\Users\<user>\github\...\target\release`.
2. O SCM passa a iniciar o servico LocalSystem desse caminho.
3. Mais tarde, malware sem elevacao no mesmo usuario substitui `mtt-search-service.exe` naquele diretorio.
4. Ao reiniciar a maquina ou o servico, o binario malicioso executa como SYSTEM.

**Recomendacao tecnica**

- Em `install_service()`, recusar instalacao se o executavel estiver em diretorio gravavel por `BUILTIN\Users`, `Authenticated Users` ou pelo usuario interativo sem exigir elevacao.
- Preferir copiar o binario do servico para uma pasta protegida antes do registro, como `{ProgramFiles}\MTT File Manager\mtt-search-service.exe`, ou aceitar apenas instalacao a partir do diretorio do instalador.
- Manter `run-console` como fluxo de desenvolvimento elevado, sem registrar SCM.
- Atualizar a documentacao para deixar claro que `install` manual a partir de `target\release` e inseguro fora de laboratorio.

**Cuidados para nao quebrar funcionalidade**

- O instalador Inno atual pode continuar usando `{app}\mtt-search-service.exe install`.
- Para dev, permitir override explicito, por exemplo `MTT_SEARCH_ALLOW_UNSAFE_SERVICE_INSTALL=1`, imprimindo aviso forte e recusando por padrao.
- Testar install/uninstall pelo instalador e `run-console` separado.

---

### SEC-02 - Verificacao do servidor do Named Pipe aceita `ERROR_ACCESS_DENIED` como prova de legitimidade

**Severidade:** media

**Modulo afetado:** [src/infrastructure/global_search.rs](../src/infrastructure/global_search.rs)

**Trecho relevante:**

```rust
Err(e) if e.code() == ERROR_ACCESS_DENIED.to_hresult() => {
    return Ok(());
}
```

**Descricao**

O cliente obtem o PID do servidor via `GetNamedPipeServerProcessId()` e tenta `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`. Se `OpenProcess` falhar com `ERROR_ACCESS_DENIED`, o codigo aceita o peer como legitimo porque, nos dois fluxos esperados, o servico real roda em integridade superior: LocalSystem ou console elevado.

Esse raciocinio e valido para o servico real, mas `ACCESS_DENIED` tambem pode ocorrer contra processo de outro usuario, outra sessao, objeto com DACL restritiva ou protecao de processo. Como o pipe do servico usa um nome global fixo, um usuario local diferente pode pre-criar o pipe quando o servico real estiver parado e o cliente pode aceitar esse servidor falso.

**Impacto / risco potencial**

Spoofing do servico de busca. O invasor nao ganha SYSTEM diretamente, mas pode:

- observar consultas enviadas pelo app ao pipe falso;
- retornar resultados falsos para a UI;
- induzir o usuario a abrir caminhos controlados pelo atacante;
- degradar a integridade das decisoes do app baseadas no status do servico.

**Cenario realista de exploracao**

1. O servico real nao esta rodando ou falhou ao criar o primeiro pipe.
2. Um usuario local malicioso cria `\\.\pipe\MTTFileManagerSearch` com DACL que permite a vitima conectar.
3. O app da vitima conecta, le o PID do servidor falso e tenta `OpenProcess`.
4. `OpenProcess` retorna `ACCESS_DENIED` por fronteira de usuario/sessao, e o cliente aceita o pipe.
5. O servidor falso devolve `SearchResponse::Results` fabricados.

**Recomendacao tecnica**

- Em modo producao, comparar o PID do pipe com o PID do servico `MTTFileManagerSearch` retornado pelo SCM (`QueryServiceStatusEx` / `SERVICE_STATUS_PROCESS`).
- Aceitar `ERROR_ACCESS_DENIED` somente se o PID tambem bater com o PID registrado do servico.
- Para modo console elevado, usar fallback restrito: SCM nao pode estar rodando o servico, o servidor do pipe deve estar na mesma sessao interativa do app e, quando `OpenProcess` funcionar, o caminho deve ser o sibling esperado do app/servico.
- Quando houver pipe squatting detectado no primeiro `CreateNamedPipeW`, considerar falha fatal do servico em vez de retry silencioso indefinido.

**Cuidados para nao quebrar funcionalidade**

- O modo servico deve continuar funcionando mesmo quando `OpenProcess` contra LocalSystem falhar.
- O modo console elevado precisa de caminho de desenvolvimento preservado; teste os dois cenarios conhecidos: servico SCM como SYSTEM e console elevado como usuario interativo.

---

### SEC-03 - `FolderSize` exposto a qualquer usuario autenticado sem autorizacao equivalente ao cliente

**Severidade:** media

**Modulos afetados:** [crates/mtt-search-service/src/ipc_server/pipe_io.rs](../crates/mtt-search-service/src/ipc_server/pipe_io.rs), [crates/mtt-search-service/src/ipc_server/handler.rs](../crates/mtt-search-service/src/ipc_server/handler.rs)

**Trechos relevantes:**

```rust
let users_sid = SidGuard::authenticated_users()?;
```

```rust
SearchRequest::FolderSize { path } => {
    // intentionally do NOT impersonate the client
    // ...
    crate::mft_reader::folder_size_for_service(&vol, frn)
}
```

**Descricao**

O DACL do pipe permite conexao de `Authenticated Users`. A busca (`Query`) e `CheckPathsModified` usam impersonacao e `CreateFileW` para validar acesso antes de revelar resultados sensiveis. `FolderSize`, por decisao funcional, nao impersona nem verifica acesso ao caminho alvo.

A justificativa funcional e importante: uma verificacao direta com `CreateFileW(GENERIC_READ, path)` ja quebrou tamanhos de pastas legitimamente visiveis como `C:\PerfLogs`. O problema de seguranca restante e que a premissa "o usuario ja esta vendo a pasta na UI" so vale para o app legitimo. Um cliente IPC arbitrario pode chamar `FolderSize` diretamente para caminhos que nunca foram exibidos na UI.

**Impacto / risco potencial**

Divulgacao local de metadados agregados: existencia aproximada, tamanho total, contagem de arquivos e contagem de subpastas de diretorios indexados. Nao ha leitura de conteudo de arquivo, mas os metadados podem revelar uso, volume de dados ou presenca de pastas sensiveis de outros usuarios.

**Cenario realista de exploracao**

Um processo nao elevado conecta ao pipe e envia:

```rust
SearchRequest::FolderSize { path: "C:\\Users\\OutroUsuario\\Documents".to_string() }
```

Se o caminho resolver no indice e os tamanhos estiverem carregados, o servico retorna `total_size`, `file_count` e `folder_count` sem provar que aquele cliente consegue listar ou abrir a pasta.

**Recomendacao tecnica**

- Nao reintroduzir o gate antigo `CreateFileW(GENERIC_READ, path)` no alvo, porque isso ja quebrou funcionalidade.
- Primeira correcao recomendada: autorizar o cliente IPC/processo para endpoints de metadados (`FolderSize`, `GetStatus` detalhado e `WarmIndex`), em vez de autorizar a subarvore do caminho. O servico pode usar `GetNamedPipeClientProcessId`, abrir o processo cliente e exigir o binario legitimo `mtt-file-manager.exe` em caminho esperado/sibling do servico instalado ou do build local.
- Manter o total de `FolderSize` como valor service-authoritative do indice MFT. Nao filtrar filhos por ACL e nao degradar para scan local de NTFS em erro de autorizacao, porque isso recria os totais errados/baixos observados anteriormente.
- Se ainda for necessario reduzir divulgacao por caminho apos o gate de cliente, usar apenas uma verificacao de visibilidade do pai (`FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES` no diretorio pai; raiz para itens na raiz), nunca `GENERIC_READ` no alvo nem caminhada ACL-recursiva.
- Retornar erro generico para `not found` e `unauthorized` quando a diferenca puder revelar existencia.

**Cuidados para nao quebrar funcionalidade**

- Testar explicitamente `C:\PerfLogs`, `C:\Windows`, `C:\Program Files`, OneDrive e caminhos NTFS normais.
- Testar em modo console elevado e modo SCM LocalSystem, porque a impersonacao/SQOS e a integridade do processo mudam entre esses cenarios.
- Preservar o fallback local para volumes nao NTFS e para misses de OneDrive ja existente no app.

---

## Analise aprofundada de regressao - SEC-02 e SEC-03

Esta secao revisa os dois achados com foco especifico em preservar o funcionamento do app normal, que roda sem elevacao, enquanto o servico de busca roda como LocalSystem ou como processo console elevado.

### SEC-02 - O que nao pode ser alterado de forma simplista

O cliente em [src/infrastructure/global_search.rs](../src/infrastructure/global_search.rs) abre `\\.\pipe\MTTFileManagerSearch`, obtem o PID do servidor por `GetNamedPipeServerProcessId()` e tenta `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`. Hoje, `ERROR_ACCESS_DENIED` e aceito como sinal de que o peer esta acima do app.

Essa aceitacao existe por um motivo funcional real:

- **Modo SCM/producao:** o servico roda como LocalSystem; o app roda como usuario comum. `OpenProcess` contra o servico pode falhar com `ACCESS_DENIED`.
- **Modo console/debug:** o servico roda no mesmo usuario, mas em terminal elevado; o app continua em integridade media. `OpenProcess` tambem pode falhar por fronteira de integridade/process policy.

Por isso, as seguintes mudancas sao regressivas:

| Mudanca proposta | Risco funcional |
| --- | --- |
| Rejeitar todo `ERROR_ACCESS_DENIED` | Pode quebrar imediatamente service mode e console mode, marcando o servico real como falso. |
| Exigir sempre token SID/path via `OpenProcess` | Quebra quando o app nao consegue abrir o processo LocalSystem/elevado. |
| Exigir sempre PID do SCM | Quebra o modo `run-console`, porque esse modo nao tem PID registrado no SCM. |
| Trocar o nome do pipe para incluir SID do usuario sem servidor multi-pipe | Pode quebrar uso por multiplos usuarios e app normal sem um mecanismo de descoberta. |

### SEC-02 - Endurecimento recomendado sem regressao

O caminho mais seguro e uma verificacao em camadas:

1. Obter `server_pid` via `GetNamedPipeServerProcessId()` como hoje.
2. Consultar o SCM para `MTTFileManagerSearch` usando `QueryServiceStatusEx(SERVICE_STATUS_PROCESS)`.
3. Se o SCM disser que o servico esta `RUNNING`, aceitar `ERROR_ACCESS_DENIED` somente quando `server_pid == dwProcessId` do SCM. Se o PID nao bater, rejeitar o pipe mesmo que o processo seja inacessivel.
4. Se o SCM nao estiver rodando o servico, tratar como modo console. Nesse caminho, preservar suporte ao console elevado, mas restringir mais: aceitar `ACCESS_DENIED` apenas quando o servidor estiver na mesma sessao interativa do app (`GetNamedPipeServerSessionId` ou `ProcessIdToSessionId`) e o primeiro pipe nao tiver um servico SCM rodando em paralelo.
5. Quando `OpenProcess` funcionar, substituir o check fraco de basename por path confiavel: `mtt-search-service.exe` deve estar no mesmo diretorio do `mtt-file-manager.exe` que esta rodando, ou no `ImagePath` registrado no SCM. Esse criterio preserva o instalador (`{app}`) e o build local (`target\release`) sem aceitar qualquer `mtt-search-service.exe` em `%TEMP%`.

Resultado esperado:

- Service mode continua funcionando quando `OpenProcess` falhar contra LocalSystem, porque o PID do SCM vira a prova de identidade.
- Console mode continua funcionando quando `OpenProcess` falhar contra o processo elevado, porque a excecao fica limitada a mesma sessao e somente quando o SCM nao esta rodando.
- Pipe squatting por outro usuario quando o servico real esta parado deixa de ser aceito apenas por `ACCESS_DENIED`.

### SEC-03 - Por que autorizacao por pasta e perigosa aqui

O app usa o servico para tamanho de pastas NTFS em [src/app/init_workers/filesystem_workers.rs](../src/app/init_workers/filesystem_workers.rs). Para NTFS, o codigo chama `global_search::folder_size()` e, se falhar, **nao** faz scan local generico; isso e proposital para evitar regressao de desempenho e valores dependentes do token do usuario. O fallback local existe para volumes nao NTFS e para misses especificos de OneDrive.

O servico responde `FolderSize` em [crates/mtt-search-service/src/ipc_server/handler.rs](../crates/mtt-search-service/src/ipc_server/handler.rs) com `folder_size_for_service()` sobre o indice MFT. Esse valor e o que evita dois problemas antigos:

- pastas como `C:\PerfLogs` retornando erro/blank por causa de `CreateFileW(GENERIC_READ)` sob impersonacao;
- totais menores que o esperado quando a subarvore contem filhos protegidos ou metadados que o usuario comum nao consegue atravessar diretamente.

As seguintes mudancas tendem a quebrar ou degradar a funcionalidade:

| Mudanca proposta | Efeito provavel |
| --- | --- |
| `current_client_can_read_path(path)` com `GENERIC_READ` no alvo | Recria a regressao de `C:\PerfLogs` e outros diretorios especiais. |
| Caminhar a subarvore sob token do usuario e somar apenas filhos acessiveis | Produz totais errados/baixos para pastas com filhos protegidos, hardlinks e areas de sistema. |
| Em erro de autorizacao, cair para `calculate_folder_size_parallel()` em NTFS | Troca o valor service-authoritative por um valor lento e token-dependente. |
| Autorizar cada resultado/filho antes de somar | Custo alto, risco de timeout e sem equivalencia com Explorer/MFT. |

### SEC-03 - Endurecimento recomendado sem afetar os totais

O melhor ponto de controle e o cliente IPC, nao a arvore de arquivos.

Implementacao sugerida no servico:

1. Criar um helper de autorizacao de cliente, por exemplo `trusted_file_manager_client(pipe)`.
2. Usar `GetNamedPipeClientProcessId(pipe)` para obter o PID do cliente.
3. Como o servico roda elevado/SYSTEM, abrir o processo cliente com `PROCESS_QUERY_LIMITED_INFORMATION` deve funcionar para o app normal.
4. Exigir imagem `mtt-file-manager.exe` em caminho confiavel:
    - mesmo diretorio de `mtt-search-service.exe` para instalacao normal e build local;
    - ou caminho explicitamente esperado pelo instalador/SCM quando disponivel.
5. Opcionalmente exigir que o token do cliente seja usuario interativo normal, nao `LocalSystem`/servico aleatorio.
6. Aplicar esse gate a `FolderSize`, status detalhado e `WarmIndex`. `Ping` pode continuar aberto para diagnostico leve.

Isso reduz o risco de cliente IPC arbitrario sem mexer no calculo do tamanho. Depois desse gate, uma verificacao adicional por caminho so deve ser considerada se o threat model exigir isolamento forte entre usuarios locais que executam o app legitimo. Se for implementada, ela deve ser limitada a uma checagem de visibilidade do pai, nao do alvo:

```text
item em C:\X\Y  -> checar list/read-attributes em C:\X
item em C:\Y    -> checar list/read-attributes em C:\
```

Mesmo essa checagem do pai precisa ser testada com cuidado em `C:\PerfLogs`, `C:\Windows`, `C:\Program Files`, OneDrive, pastas reparse/cloud e maquinas com antivirus agressivo. Ela pode impedir resposta, mas nao deve alterar o total retornado. Se falhar, o app deve mostrar tamanho indisponivel, nao recalcular NTFS localmente.

### Matriz minima de teste antes de corrigir SEC-02/SEC-03

| Cenario | O que validar |
| --- | --- |
| Servico SCM como LocalSystem + app normal | `Ping`, `GetStatus`, `Query`, `CheckPathsModified` e `FolderSize` continuam funcionando mesmo se `OpenProcess` contra o servico falhar. |
| Servico console elevado + app normal | `verify_server_process` aceita o servico real sem SCM PID e sem exigir `OpenProcess` bem-sucedido. |
| Pipe falso criado antes do servico por outro usuario | App rejeita o pipe; nao deve aceitar somente por `ACCESS_DENIED`. |
| Pipe falso criado pelo mesmo usuario com exe renomeado | App rejeita se o caminho nao for o sibling esperado do app/servico. |
| `C:\PerfLogs` | `FolderSize` retorna valor service-authoritative ou indisponivel por motivo real, mas nao falha por `GENERIC_READ` no alvo. |
| `C:\Windows` e `C:\Program Files` | Totais nao ficam sistematicamente baixos por filtragem ACL de filhos. |
| OneDrive NTFS com miss no indice | Fallback estreito por `Path not found in index` continua funcionando. |
| Volume nao NTFS | Continua usando scan local, sem depender do servico. |

### Conclusao da analise aprofundada

Para SEC-02, a correcao segura nao e remover `ACCESS_DENIED`, e sim dar contexto a ele: PID do SCM em producao e fallback restrito para console mode. Para SEC-03, a correcao segura nao e autorizar a pasta nem filtrar a subarvore; e autorizar o cliente IPC e preservar o calculo MFT service-authoritative. Essa distincao e essencial para evitar exatamente as regressoes historicas: app comum deixando de falar com servico elevado e tamanho de pasta ficando errado por permissao.

---

### SEC-04 - Ausencia de rate limit por usuario/processo no IPC do servico elevado

**Severidade:** media

**Modulos afetados:** [crates/mtt-search-service/src/ipc_server/mod.rs](../crates/mtt-search-service/src/ipc_server/mod.rs), [crates/mtt-search-service/src/ipc_server/handler.rs](../crates/mtt-search-service/src/ipc_server/handler.rs), [crates/mtt-search-service/src/ipc_authorization.rs](../crates/mtt-search-service/src/ipc_authorization.rs)

**Descricao**

O servico ja limita `MAX_ACTIVE_CLIENTS` a 8, limita payloads a 64 KiB, limita batches de autorizacao e usa deadline de 6 segundos para busca autorizada. Isso reduz muito o risco. Ainda assim, qualquer usuario autenticado local pode manter as 8 conexoes ocupadas e forcar trabalho pesado em processo elevado:

- consultas amplas de um caractere ou termos comuns, que percorrem indices grandes;
- autorizacao repetida por `CreateFileW` em muitos pais de resultados;
- chamadas `FolderSize` em muitas subarvores;
- reparo de tamanho zero em `repair_suspicious_zero_folder_size()`, que pode abrir volume e consultar MFT para ate 4096 FRNs por requisicao.

**Impacto / risco potencial**

Negacao de servico local contra o servico de busca e degradacao do app. Em maquinas com indices grandes, um processo malicioso pode consumir CPU, I/O de MFT e os slots de handler do pipe, afetando todos os usuarios.

**Cenario realista de exploracao**

Malware sem elevacao abre varias conexoes em paralelo, envia consultas muito amplas (`"a"`, `"e"`, nomes comuns) com offsets variados e repete `FolderSize` em caminhos grandes. Mesmo com os deadlines atuais, o atacante pode manter o servico continuamente ocupado.

**Recomendacao tecnica**

- Adicionar rate limit por SID do cliente e, se possivel, por PID via `GetNamedPipeClientProcessId` / token do cliente impersonado.
- Aplicar cotas separadas para `Query`, `FolderSize`, `WarmIndex` e reparo de tamanho zero.
- Cachear resultados de autorizacao negativa por conexao e considerar cache curto por SID/caminho.
- Colocar cooldown por `(drive_letter, dir_frn)` para `repair_suspicious_zero_folder_size()`.
- Considerar rejeitar ou degradar consultas de um unico caractere quando o chamador exceder a cota, sem remover suporte do app legitimo.

**Cuidados para nao quebrar funcionalidade**

- O usuario legitimo pode digitar busca incremental rapidamente; usar token bucket com burst pequeno em vez de cooldown fixo agressivo.
- A UI deve receber erro transitivo claro quando houver throttling, sem marcar o servico como offline.

---

### SEC-05 - `GetStatus` revela metadados de volumes por padrao a qualquer cliente do pipe

**Severidade:** baixa

**Modulos afetados:** [crates/mtt-search-service/src/ipc_server/handler.rs](../crates/mtt-search-service/src/ipc_server/handler.rs), [crates/mtt-search-service/src/security_policy.rs](../crates/mtt-search-service/src/security_policy.rs)

**Descricao**

`SearchRequest::GetStatus` nao impersona nem diferencia o cliente. Por padrao, `IpcSecurityPolicy::redact_status_metrics` e falso, entao qualquer cliente autenticado local pode obter letras de drives indexados, estados, contagem de arquivos indexados e progresso.

Os detalhes de erro e o caminho completo do executavel ja sao redigidos, o que e bom. O restante ainda e informacao de reconhecimento do sistema.

**Impacto / risco potencial**

Divulgacao de topologia local: volumes presentes, progresso de indexacao, quantidade aproximada de arquivos por volume e atividade recente de indexacao. O impacto isolado e baixo, mas ajuda um atacante local a escolher alvos e sincronizar abuso do servico.

**Cenario realista de exploracao**

Um processo local consulta `GetStatus` periodicamente para descobrir quando o indice do volume `C:` esta pronto e quantos arquivos existem, antes de iniciar consultas amplas ou `FolderSize`.

**Recomendacao tecnica**

- Redigir metricas por padrao para clientes nao verificados.
- Alternativamente, impersonar e retornar apenas volumes cujas raizes o cliente consegue consultar.
- Manter uma opcao de diagnostico explicita para dev/admin (`MTT_SEARCH_REDACT_STATUS_METRICS=0` ou equivalente), em vez de expor detalhes por padrao.

**Cuidados para nao quebrar funcionalidade**

- A UI precisa saber se o servico esta pronto; retornar estados coarse-grained (`ready`, `scanning`, `error`) sem contagens pode ser suficiente.
- Se a UI atual usa contagens para progresso visual, manter contagens apenas para app verificado ou atras de preferencia local.

---

### SEC-06 - Extracao de arquivos compactados sem limite de bytes extraidos por entrada/operacao

**Severidade:** media

**Modulo afetado:** [src/infrastructure/archive_extract.rs](../src/infrastructure/archive_extract.rs)

**Trechos relevantes:**

```rust
io::copy(&mut entry, &mut out_file)?;
io::copy(reader, &mut out_file)?;
let (data, next) = header.read()?;
fs::write(&dest_path, &data)?;
```

**Descricao**

O modulo de extracao tem boas defesas contra path traversal: normaliza separadores, remove `.`/`..`, sanitiza nomes reservados do Windows e valida que o destino permanece abaixo da pasta escolhida. O risco restante e DoS por tamanho: nao ha limite explicito de bytes descomprimidos por entrada nem de total extraido por operacao.

ZIP, 7z e TAR fazem streaming para disco sem limite. RAR le a entrada inteira para memoria (`header.read()`) antes de gravar, o que aumenta o risco de exaustao de memoria para uma entrada RAR grande selecionada.

**Impacto / risco potencial**

Negacao de servico no contexto do usuario: consumo de disco, memoria e CPU ao extrair arquivo malicioso com taxa de compressao alta ou metadado de tamanho enorme. Nao ha elevacao de privilegio direta.

**Cenario realista de exploracao**

Um arquivo compactado aparentemente pequeno contem uma entrada selecionada que expande para dezenas/centenas de GB. Ao copiar de dentro do arquivo pelo file manager, o app escreve ate encher disco ou consumir memoria no caso RAR.

**Recomendacao tecnica**

- Adicionar limites configuraveis: tamanho maximo por entrada e tamanho total por operacao.
- Usar metadados quando disponiveis: `ZipFile::size()`, tamanho TAR do header, tamanho descomprimido de 7z/RAR quando exposto pela crate.
- Envolver readers em contador de bytes e abortar quando exceder o limite, mesmo se o metadado estiver ausente ou mentir.
- Para RAR, evitar `header.read()` para entradas grandes se a crate permitir streaming; caso contrario, recusar acima de limite antes de ler.

**Cuidados para nao quebrar funcionalidade**

- Definir limite padrao alto o bastante para uso real e oferecer confirmacao/override para arquivos grandes.
- Garantir limpeza de arquivo parcial quando a extracao for abortada por limite.

---

### SEC-07 - Mutex global fixo do image viewer permite DoS local de instancia unica

**Severidade:** baixa

**Modulo afetado:** [src/image_viewer/mod.rs](../src/image_viewer/mod.rs)

**Trecho relevante:**

```rust
const IMAGE_VIEWER_MUTEX_NAME: &str = "Global\\MTTFileManager_ImageViewer_SingleInstance\0";
let handle = CreateMutexW(None, true, PCWSTR(wide.as_ptr())).ok()?;
```

**Descricao**

O image viewer usa mutex global de nome fixo e DACL padrao. Se outro processo conseguir criar esse mutex antes do viewer, `try_acquire()` falha, o processo tenta encaminhar via IPC e encerra. O pipe do viewer em si e bem mais restrito: DACL por SID do usuario atual + SYSTEM, `PIPE_REJECT_REMOTE_CLIENTS` e `FILE_FLAG_FIRST_PIPE_INSTANCE`.

**Impacto / risco potencial**

Negacao de servico local do visualizador de imagem. Nao ha execucao de codigo nem elevacao.

**Cenario realista de exploracao**

Processo malicioso cria `Global\MTTFileManager_ImageViewer_SingleInstance` e nao cria o pipe legitimo. Ao abrir imagem, o viewer acredita que ja existe instancia, o forward IPC falha e a janela nao abre.

**Recomendacao tecnica**

- Usar `Local\` em vez de `Global\`, porque o viewer e por sessao/usuario.
- Incluir hash do SID do usuario no nome do mutex.
- Criar mutex com security descriptor explicito, alinhado ao pipe do viewer.

**Cuidados para nao quebrar funcionalidade**

- A instancia unica deve continuar funcionando para multiplas janelas do mesmo usuario.
- Nao precisa funcionar entre usuarios/sessoes diferentes.

## Superficies revisadas sem vulnerabilidade confirmada

### Execucao de processos e comandos

- Viewers usam `std::env::current_exe()` e `Command::new(exe).arg(...)`, sem shell textual. Modulos: [src/image_viewer/mod.rs](../src/image_viewer/mod.rs), [src/pdf_viewer/mod.rs](../src/pdf_viewer/mod.rs), [src/text_viewer/mod.rs](../src/text_viewer/mod.rs), [src/video_player/mod.rs](../src/video_player/mod.rs).
- `attrib.exe` e resolvido via `GetSystemDirectoryW`, nao por `PATH`/`SYSTEMROOT`. Modulo: [src/infrastructure/onedrive/pin_state.rs](../src/infrastructure/onedrive/pin_state.rs).
- `ffmpeg.exe` e procurado apenas ao lado do executavel ou via `MTT_FFMPEG_PATH` absoluto existente, nao por `PATH`/CWD. Modulo: [src/infrastructure/media/hardware_acceleration.rs](../src/infrastructure/media/hardware_acceleration.rs).
- Helper elevado de renomear volume usa `ShellExecuteExW` com `runas`, executa o proprio binario e valida drive/label. A citacao em `SEC-01` continua aplicavel: esse padrao so e seguro se o executavel instalado estiver em caminho protegido.

### DLL hijacking / search order

- App principal e servico chamam `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)` no inicio. Modulos: [src/main.rs](../src/main.rs), [crates/mtt-search-service/src/main.rs](../crates/mtt-search-service/src/main.rs).
- Pdfium e carregado do diretorio do executavel antes do fallback de sistema, e o build/installer verificam hash do DLL. Modulos: [src/pdf_viewer/renderer.rs](../src/pdf_viewer/renderer.rs), [build.rs](../build.rs), [installer/build_installer.ps1](../installer/build_installer.ps1).
- MPV config e procurado apenas relativo ao diretorio do executavel, nao no CWD. Modulo: [src/video_player/mod.rs](../src/video_player/mod.rs).

### Deserializacao e framing IPC

- `bincode` usa limite igual ao tamanho real do buffer em `decode_message()`. Modulo: [crates/mtt-search-protocol/src/lib.rs](../crates/mtt-search-protocol/src/lib.rs).
- Servico rejeita payload IPC maior que 64 KiB e o cliente rejeita resposta maior que 1 MiB. Modulos: [crates/mtt-search-service/src/ipc_server/pipe_io.rs](../crates/mtt-search-service/src/ipc_server/pipe_io.rs), [src/infrastructure/global_search.rs](../src/infrastructure/global_search.rs).
- `SearchRequest::validate()` limita texto de busca, limite de resultados e quantidade de paths em `CheckPathsModified`.

### Persistencia do indice elevado

- `C:\ProgramData\MTT-File-Manager` e hardcoded, nao vem de `%PROGRAMDATA%`.
- O diretorio e aberto com `FILE_FLAG_OPEN_REPARSE_POINT`, validado contra junction/reparse point e endurecido por handle com DACL protegida.
- Indices binarios usam HMAC-SHA256 com chave DPAPI LocalMachine; mismatch causa descarte e rebuild.

### Path traversal em operacoes de arquivo

- `sanitize_path()` bloqueia NUL, `.`/`..`, ADS (`:`), nomes reservados e, por padrao, reparse points antes e depois da canonicalizacao. Modulo: [src/infrastructure/security.rs](../src/infrastructure/security.rs).
- Operacoes de arquivo permitem symlinks por compatibilidade com Windows, mas ainda fazem validacao de drive e componentes.
- Extracao de arquivos compactados remove traversal e nomes reservados; o achado restante e limite de tamanho, nao escape de destino.

### Shell namespace e contexto do Explorer

O app deliberadamente usa Shell APIs e `IContextMenu` para comportamento de file manager. Isso e superficie de execucao no contexto do usuario, especialmente se o usuario invocar verbos de shell ou abrir arquivos executaveis/atalhos. Nao identifiquei isso como vulnerabilidade do app porque depende de acao do usuario e segue o modelo de um gerenciador de arquivos, mas deve continuar fora do contexto do servico elevado.

## Plano de acao recomendado

### Prioridade 0 - Corrigir risco de elevacao via instalacao do servico

1. Bloquear `install_service()` quando `current_exe()` estiver sob perfil de usuario, repo, `Downloads`, `Desktop`, `%TEMP%` ou qualquer diretorio cujo ACL permita escrita a usuarios nao administradores.
2. Preferir instalacao/copia para caminho protegido pelo instalador.
3. Atualizar README e docs de build para recomendar `run-console` em dev e instalador para SCM.
4. Testar instalacao via Inno, uninstall e upgrade.

### Prioridade 1 - Endurecer confianca do Named Pipe do servico

1. No cliente, validar PID do pipe contra o PID do servico retornado pelo SCM em modo producao.
2. Tratar `ERROR_ACCESS_DENIED` como sucesso somente quando houver contexto confiavel: PID do SCM ou fallback console restrito por sessao.
3. Fortalecer o caminho `OpenProcess` bem-sucedido exigindo path confiavel, nao apenas basename/SID.
4. Adicionar teste/manual harness com pipe falso pre-criado por mesmo usuario e por outro usuario.

### Prioridade 1 - Reduzir divulgacao de metadados via IPC

1. Implementar gate de cliente IPC/processo para `FolderSize`, `GetStatus` detalhado e `WarmIndex`, validando `mtt-file-manager.exe` em caminho confiavel.
2. Manter `FolderSize` service-authoritative; nao filtrar subarvore por ACL e nao cair para scan local NTFS em erro de autorizacao.
3. Se ainda necessario, avaliar checagem de visibilidade do pai apenas depois do gate de cliente, evitando `GENERIC_READ` no alvo.
4. Redigir ou reduzir `GetStatus` para clientes nao verificados.
5. Garantir erros indistinguiveis para `not found` versus `unauthorized` onde isso revelar existencia.
6. Revalidar `C:\PerfLogs`, OneDrive e pastas protegidas para evitar regressao.

### Prioridade 2 - Controles anti-DoS locais

1. Adicionar rate limit por SID/PID no IPC.
2. Adicionar cooldown por path/FRN para reparo de tamanho zero.
3. Manter os limites existentes de payload, cliente e deadline.
4. Adicionar metricas/logs de throttling sem vazar caminhos sensiveis.

### Prioridade 2 - Limites de extracao de arquivo

1. Definir limite por entrada e total por operacao.
2. Implementar contador de bytes em streaming e limpeza de parciais.
3. Evitar leitura RAR inteira em memoria para entradas grandes ou recusar por limite.

### Prioridade 3 - Hardening menor

1. Trocar o mutex global do image viewer por nome `Local\` + SID/hash + DACL explicita.
2. Considerar fail-fast se `SetDefaultDllDirectories` falhar no servico, pelo menos em producao.
3. Documentar shell verbs/abertura de arquivos como superficie intencional no contexto do usuario, nunca no servico.

## Checklist de regressao para as correcoes

- Servico SCM como LocalSystem: app normal nao elevado consegue `Ping`, `Query`, `CheckPathsModified` e `FolderSize` em pastas normais.
- Console mode elevado: app normal nao elevado continua funcionando apesar de diferenca de integridade.
- `FolderSize` nao quebra `C:\PerfLogs`, `C:\Windows`, `C:\Program Files`, OneDrive e volumes NTFS grandes.
- Usuario local diferente nao consegue obter status detalhado nem `FolderSize` de outro usuario.
- Pipe falso pre-criado nao e aceito pelo cliente em producao.
- Instalacao manual a partir de `target\release` em perfil de usuario e recusada por padrao.
- Extracao de ZIP/7z/RAR/TAR preserva traversal blocking e aborta limpo ao exceder limite.
