# AliExpress Macropad Configurator · Macro para Mini Teclados

Configurador Rust para Windows de mini teclados USB programáveis vendidos no AliExpress como **MINI KeyBoard**, **CH57x macro keypad** ou **mini teclado de macros com botão giratório**. Programe atalhos, mídia, mouse e iluminação em uma interface visual, com indicação do controle acionado.

**Windows x64** · [Baixar o ZIP na página de versões](https://github.com/soldieg/aliexpress-macropad-configurator/releases/latest) · [Guia em português](LEIA-ME.txt) · [English](#english)

**Prévia Linux e macOS** · [Pacotes dos testes nativos](https://github.com/soldieg/aliexpress-macropad-configurator/actions/workflows/ports-preview.yml?query=branch%3Apreview%2Flinux-macos) · [Instalação, compatibilidade e testes pendentes](docs/platforms.md)

## Recursos

- Configure teclas, cliques e os sentidos dos seletores giratórios. Identifique o aparelho USB e confira o perfil antes de gravar.
- Crie, mova, dimensione e exclua controles ou layouts visuais. O mapa reage às teclas físicas identificáveis, com animação por passo do giratório.
- Grave ações nativas de volume no mini teclado. O modo **Passo do aplicativo** ajusta de 1 a 10 pontos percentuais por giro e mostra o resultado em um indicador do aplicativo, inclusive na bandeja.
- Configure gestos para ligar ou apagar a tela e abrir uma contagem cancelável de cinco segundos para desligamento. O primeiro gesto de desligamento é um teste.
- Ajuste os modos de LED disponíveis no aparelho. Perfis compatíveis com WebHub também oferecem cor RGB, brilho, velocidade e leitura.
- Use o idioma do Windows ou escolha uma das 16 opções. A interface usa português brasileiro e oferece 15 traduções.

| Aparelho | Suporte |
| --- | --- |
| `1189:8890` Legacy (3 teclas + 1 giratório) | Verificado em um aparelho físico. O mesmo VID:PID também aparece em outras geometrias. |
| `1189:8840` FD (6 teclas + 1 giratório, MI_00) | Teclado e rolagem implementados a partir do protocolo. Validação física pendente. |
| `6D7B:DCFA` WebHub (MI_02) | Ações e RGB implementados a partir do protocolo. Validação física pendente. |
| Outras variantes RSoft Legacy/Extended | Confira o protocolo e a interface em **Identificar USB**. O desenho disponível independe da geometria que o firmware aceita. |

No firmware Legacy documentado, o comando de volume nativo envia um evento de mídia por giro. O Windows escolhe o tamanho desse passo. O ajuste exato depende do processo na bandeja. Combinações de vários botões físicos também precisam do aplicativo.

## English

A Rust Windows configurator for programmable USB macropads and one-handed mini keyboards commonly sold as **AliExpress MINI KeyBoard**, **CH57x USB macro keypad**, **3-key macro pad with rotary encoder**, or **programmable shortcut keypad with a volume knob**. It supports keyboard, mouse and media mappings, visual layout editing, hardware identification, rotary volume and scroll controls, and host-side shortcuts for display and shutdown. [Compatibility details and Linux/macOS assessment](docs/platforms.md).

## Idiomas e identificação

[Vocabulário por idioma e referências de identificação](docs/identification.md). A etiqueta da loja e o VID:PID ajudam a encontrar o protocolo. Confira também a interface USB e a geometria física.

Mensagens e nomes fornecidos pelo Windows ou pelo próprio aparelho podem aparecer no idioma de origem. O renderizador ainda tem limitações em algumas ligaduras indianas.

## Compilação

No Windows x64, com Rust estável e ferramentas MSVC:

```sh
cargo build --release --locked
cargo test --locked
```

Para informação sobre USB sem abrir a interface, execute `mini-teclado-usb.exe --diagnostico`.

## Licença e créditos

GPL-3.0-only. Configuração e formatos RSoft adaptados de [rOzzy1987/MacroPad](https://github.com/rOzzy1987/MacroPad/tree/c51d4d39bc94ec4c0cdfc6a2b8203551a852dca5). Referências de protocolo [WebHub de ysalitrynskyi](https://github.com/ysalitrynskyi/MacroPad), [Knurl](https://github.com/dozzenn/knurl) e [variante FD de coldmon91](https://github.com/coldmon91/MacroPad2). AliExpress é mencionado para identificar a origem comum dos aparelhos.
