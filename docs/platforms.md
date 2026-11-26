# Prévia para Linux e macOS

O configurador usa a mesma interface Rust/egui nas três plataformas. Layouts, editor, animações, iluminação conforme o aparelho, atalhos da janela e 16 opções de idioma são compartilhados. Os pacotes desta branch são experimentais.

| Recurso | Linux | macOS |
| --- | --- | --- |
| Configuração USB | HIDAPI por hidraw, com identificação pelo número de interface. | HIDAPI por IOKit, com identificação pelo número de interface. |
| Captura e ações em segundo plano | evdev do mini teclado selecionado. Para repetir ações individuais de teclas que participam de combinações, o app usa uinput. | IOHIDManager observa o mini teclado selecionado. Quartz reproduz teclado e mouse. O Core Audio reproduz volume e mudo quando a saída aceita esses controles. |
| Volume e rolagem | Passo ajustável de 1 a 10 pontos percentuais via PipeWire/WirePlumber e indicador do app. Rolagem e mídia em modo direto usam os comandos gravados no firmware. | Passo ajustável de 1 a 10 pontos percentuais via Core Audio e indicador do app. Rolagem e mídia em modo direto usam os comandos gravados no firmware. |
| Tela, desligamento e cancelamento | X11/DPMS, KDE Wayland via `kscreen-doctor` ou Wayland com protocolo compatível com `wlopm`. O pedido de desligamento usa systemd. | Repouso de tela via `pmset` e despertar via IOKit. Após a contagem, o macOS recebe um pedido para abrir a confirmação nativa de desligamento. |
| Bandeja e início de sessão | Menu StatusNotifier em ambientes que o oferecem. Início de sessão via XDG Autostart. | Menu na barra de menus e início de sessão por LaunchAgent do usuário. |

## Linux x86-64

Extraia o pacote e mantenha o executável em um caminho permanente se for ativar o início automático. A compilação do pacote usa Ubuntu 24.04. No X11, instale `libxkbcommon-x11-0` se a biblioteca estiver ausente. Para permitir acesso ao dispositivo pela sessão atual, instale a regra incluída. Ela se restringe aos VID/PID listados no arquivo.

```sh
sudo install -m 0644 70-mini-teclado-usb.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
```

Reconecte o mini teclado. Execute `./mini-teclado-usb --diagnostico` para conferir a identificação e o acesso ao relatório USB. O volume por passo do aplicativo usa `wpctl`. A reprodução de ações conciliadas com combinações exige permissão de gravação em `/dev/uinput`, administrada pela distribuição ou pelo responsável pela máquina.

## macOS Apple Silicon e Intel

Escolha o ZIP correspondente ao processador. Extraia e mova o app para Aplicativos antes de autorizar ou habilitar o início de sessão. O pacote de prévia tem assinatura ad hoc. A distribuição por Developer ID e a notarização estão pendentes.

Para as combinações, conceda **Monitoramento de Entrada** ao app. A reprodução de teclado e mouse requer **Acessibilidade**. O pedido de desligamento pode exigir autorização em **Automação**. Para diagnóstico em Terminal, execute o binário dentro de `Contents/MacOS` com `--diagnostico`.

## O que falta validar

- Teste com o USB físico `1189:8890` em cada sistema. Teste identificação, gravação e restauração de uma ação, rolagem, passos do dial, combos, reabertura da tela e cancelamento da contagem. Use a simulação no primeiro teste.
- Confirme a bandeja, início automático, pop-up em segundo plano e ativação do volume nas saídas de áudio e sessões gráficas pretendidas. O indicador do aplicativo acompanha o passo gerenciado.
- No macOS, anterior, próximo e reprodução/pausa continuam disponíveis para gravação direta no firmware. A repetição dessas ações pelo aplicativo, para uma tecla compartilhada por uma combinação, está pendente. O aplicativo recusa ativar essa associação antes de substituir a ação gravada.
- A interface FD do `1189:8840` ainda precisa de um transporte validado no Linux e no macOS. Extended e WebHub também precisam de validação física nessas plataformas.
- O firmware Legacy transmite o volume de mídia usando o passo do sistema com o aplicativo fechado. O passo exato e as combinações que comandam o sistema exigem que o processo fique rodando. Ao trocar de sistema, restaure modo direto no perfil de origem se o perfil de destino ainda não tiver as ações individuais salvas.

Referências [HIDAPI](https://github.com/libusb/hidapi) · [Linux hidraw](https://www.kernel.org/doc/html/latest/hid/hidraw.html) · [Linux uinput](https://www.kernel.org/doc/html/latest/input/uinput.html) · [WirePlumber wpctl](https://pipewire.pages.freedesktop.org/wireplumber/man/wpctl.html) · [IOHIDManager](https://developer.apple.com/documentation/iokit/1438367-iohidmanagerregisterinputvalueca) · [Core Audio](https://developer.apple.com/documentation/coreaudio).
