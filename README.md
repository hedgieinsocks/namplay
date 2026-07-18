<img src="data/io.github.hedgieinsocks.Namplay.png" width="128" height="128">

# Namplay

GTK4/Libadwaita app to run A2 [Neural Amp Modeler](https://github.com/sdatkinson/neural-amp-modeler) profiles via PipeWire (JACK)

![Screenshot](assets/screenshot.png)

## ✨ Features

* Noise Gate
* 3-band EQ with High/Low Pass
* Pedal NAM Profile
* Amp/Rig NAM Pofile
* Impulse Response
* Presets

## 📥 Installation

```sh
# install JACK implementation based on PipeWire e.g. on Fedora
❯ sudo dnf install pipewire-jack-audio-connection-kit
# download latest flatpak release
❯ curl -sLO https://github.com/hedgieinsocks/namplay/releases/download/v0.2.4/io.github.hedgieinsocks.Namplay.flatpak
# install it
❯ sudo flatpak install io.github.hedgieinsocks.Namplay.flatpak
```

## 📖 Usage

1. Configure PipeWire (JACK)

```sh
❯ cat ~/.config/pipewire/jack.conf.d/jack.conf
jack.properties = {
  node.latency = 256/48000
}
```

2. Launch Namplay
3. Use e.g. https://github.com/dp0sk/Crosspipe (or `pw-link`) to connect Namplay node to your guitar interface and the output sink
4. Select a profile downloaded from https://www.tone3000.com/
5. Jam!

## 📜 License

[MIT](LICENSE)

## 🔈 AI Transparency & Attributions

This project is **vibe-coded** for my personal use. I'm not an audio engineer and don't write in rust, but I try to whip AI to keep it as simple and tight as possible. So far it meets my humble aesthetical and functional needs. Hopefully, you will find it useful as well.

* inspired by https://github.com/brummer10/NeuralRack
* made possible with https://github.com/OpenSauce/nam-rs
* implemented via https://github.com/anthropics/claude-code
* icon by [Freepik](https://www.flaticon.com/authors/freepik) from [Flaticon](https://www.flaticon.com/)
