---
title: WhatsApp para Linux
description: O WhatsApp não tem app para Linux. O ZapFast é um cliente nativo de WhatsApp para Linux, macOS e Windows, em Rust, sem navegador por dentro. Abre em menos de um segundo.
permalink: /pt-br/
lang: pt-BR
seo:
  title: "WhatsApp para Linux: um app nativo"
alternates:
  - lang: en
    url: /whatsapp-on-linux/
  - lang: pt-BR
    url: /pt-br/
---

O WhatsApp tem apps para celular, Windows e macOS, e nenhum para Linux. No
Linux, sobram três caminhos:

- **O WhatsApp Web numa aba do navegador.** Funciona em qualquer lugar, mas é
  uma aba: fecha junto com o navegador, e a página com os processos do
  navegador ocupa cerca de um gigabyte de memória.
- **Um app que embrulha o WhatsApp Web.** Dá ao WhatsApp uma janela e um
  ícone na bandeja, mas por dentro continua sendo a página web com um motor de
  navegador.
- **Um cliente nativo como o ZapFast.** Ele fala diretamente o protocolo de
  dispositivos conectados do WhatsApp, sem página web e sem navegador.

O ZapFast é um único programa escrito em Rust. No nosso teste no Linux, ele
abre em menos de um segundo e usa cerca de 200 MB de memória, contra 1,13 GB
do WhatsApp Web com os processos do Chromium.
[Veja as medições](/benchmarks/) (em inglês).

<img class="VPImage dark" src="{{ '/screenshot.png' | relative_url }}" alt="O ZapFast mostrando uma conversa com um anexo, mensagens de voz, reações, uma resposta citada e uma prévia de link" width="1800" height="1360">
<img class="VPImage light" src="{{ '/screenshot-light.png' | relative_url }}" alt="O ZapFast mostrando uma conversa com um anexo, mensagens de voz, reações, uma resposta citada e uma prévia de link" width="1800" height="1360">

A maior parte do app já está em português do Brasil, e ele segue o idioma
do seu sistema.

## Como instalar

Todos os pacotes estão na [página de download](/download/#linux) (em inglês).
Em resumo:

| Distribuição | Como |
| --- | --- |
| Arch, Manjaro, CachyOS, Omarchy | `yay -S zapfast-bin`, pelo AUR |
| Fedora | `sudo dnf copr enable paolino/zapfast` e depois `sudo dnf install zapfast` |
| Debian, Ubuntu, Linux Mint, Pop!_OS | o `.deb` da página de download: `sudo apt install ./zapfast_*_amd64.deb` |
| Outras distribuições com RPM | o `.rpm` da página de download |
| NixOS e Nix | `nix profile install github:crmne/zapfast`, ou o módulo do Home Manager |
| Qualquer distribuição | o pacote Flatpak ou o AppImage |

Também há versões para [macOS e Windows](/download/).

Depois, abra o ZapFast e conecte pelo celular, em **Dispositivos
conectados**, com o QR code ou com um código enviado ao seu número.

## O que ele faz

- **Suas conversas guardadas no computador.** As mensagens ficam num arquivo
  criptografado, com a chave no chaveiro do sistema, e as mais antigas vêm do
  celular conforme você rola a conversa.
- **Chamadas de voz e de vídeo.** Faça e atenda chamadas individuais,
  escolhendo microfone, alto-falante e câmera.
- **Continua aberto.** Fechar a janela deixa o ZapFast na bandeja, e as
  notificações abrem a conversa na mensagem que anunciaram.
- **Com a cara do seu desktop.** Tema claro, escuro ou o do sistema, temas de
  cores próprios e, no Omarchy, o tema do seu desktop.
- **Vários números.** Conecte mais de um número de WhatsApp e alterne entre
  eles na mesma janela.

## Perguntas

### Dá para instalar o WhatsApp no Linux?

O app oficial do WhatsApp, não: ele não existe. Dá para usar o WhatsApp Web
no navegador ou instalar um cliente como o ZapFast, que se conecta à sua conta
do mesmo jeito que o WhatsApp Web.

### Como usar o WhatsApp no Ubuntu?

Baixe o `.deb` na [página de download](/download/#linux), instale com
`sudo apt install ./zapfast_*_amd64.deb`, abra o ZapFast pelos seus apps e
escaneie o QR code com o celular em **Dispositivos conectados**.

### O celular precisa ficar ligado?

Não. O ZapFast é um dispositivo conectado, como o WhatsApp Web, então envia e
recebe mensagens com o celular desligado. O histórico mais antigo vem do
celular, então buscar mensagens além das que o ZapFast já guardou espera o
celular ficar online.

### É gratuito?

Sim. O ZapFast é código aberto, sob a licença MIT, e o código está
[no GitHub](https://github.com/crmne/zapfast).

### É seguro para a minha conta?

O ZapFast é um cliente não oficial, e usar um pode ir contra os termos do
WhatsApp. Ele se conecta como um dispositivo comum e nunca automatiza nem
envia mensagens em massa, mas o WhatsApp ainda pode restringir contas que
usam clientes não oficiais.

O ZapFast é um projeto independente, sem vínculo com o WhatsApp LLC ou a Meta.
WhatsApp é uma marca registrada do WhatsApp LLC.
