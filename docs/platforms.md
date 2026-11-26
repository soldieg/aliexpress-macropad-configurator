# Linux e macOS

A interface e boa parte do código de configuração podem ser reaproveitadas. Esta versão entrega um executável para Windows. As colunas abaixo descrevem caminhos de implementação e validação para os outros sistemas.

| Recurso | Linux | macOS |
| --- | --- | --- |
| Interface, layouts, traduções e lógica dos gestos | Reaproveitar o código Rust/egui e habilitar Wayland e X11. Adaptar fontes, tema inicial e idioma do sistema. | Reaproveitar o código Rust/egui. Usar as fontes e preferências do macOS. |
| Detecção e gravação USB | Usar HIDAPI/hidraw e regras udev restritas às interfaces verificadas. Usar o número da interface USB, sem depender do formato do caminho Windows. | Usar HIDAPI/IOKit e validar o relatório, a interface e as permissões. O Knurl confirma a viabilidade para parte das famílias. |
| Ações e combinações globais | Capturar entrada por dispositivo via evdev. A reprodução em Wayland depende do compositor e das permissões do portal ou de uinput. | Usar IOHIDManager por aparelho e Controle de Entrada. A reprodução de eventos usa APIs Quartz e pode exigir Acessibilidade. |
| Volume exato e indicador | PipeWire/WirePlumber ou backend equivalente, com indicador próprio. No GNOME, o passo dos comandos nativos também pode ser definido globalmente. | Usar Core Audio quando a saída expõe volume ajustável, com indicador próprio. A saída digital pode não expor esse controle. |
| Tela, desligamento e cancelamento | X11 permite DPMS. O controle da tela em Wayland varia por compositor. O desligamento passa por logind e respeita autorizações e inibidores. | Usar a integração de energia do sistema e lidar com confirmação, permissões e aplicativos que podem interromper o desligamento. |
| Bandeja e início automático | Validar StatusNotifier e o ambiente de desktop. Início de sessão por XDG Autostart. | Usar NSStatusItem e SMAppService, com empacotamento, assinatura e teste em máquina real. |

O port pode preservar a mesma tela do configurador. A paridade das ações globais exige testes por ambiente e os modelos Legacy, Extended, FD e WebHub precisam validar a escrita em cada sistema.

Referências primárias [HIDAPI](https://github.com/libusb/hidapi) · [Linux hidraw](https://www.kernel.org/doc/html/latest/hid/hidraw.html) · [Portal RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html) · [WirePlumber wpctl](https://pipewire.pages.freedesktop.org/wireplumber/man/wpctl.html) · [GNOME media keys](https://github.com/GNOME/gnome-settings-daemon/blob/main/data/org.gnome.settings-daemon.plugins.media-keys.gschema.xml.in) · [systemd-logind](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.login1.html) · [Knurl para macOS](https://github.com/dozzenn/knurl) · [IOHIDManager da Apple](https://developer.apple.com/documentation/iokit/1438367-iohidmanagerregisterinputvalueca) · [Core Audio da Apple](https://developer.apple.com/documentation/audiotoolbox/kaudiohardwareservicedeviceproperty_virtualmainvolume).
