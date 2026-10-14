#!/usr/bin/env python3
"""Construit le modèle Parakeet de Lumen (texte des transcriptions, 24 langues
européennes), publié dans la version `parakeet-1` du dépôt lumen.

Point de départ : Parakeet TDT 0.6B v3 de NVIDIA (CC BY 4.0), exporté en ONNX
en pleine précision par sherpa-onnx (2,5 Go). Seuls les poids des produits de
matrices de l'encodeur sont compressés, en 8 bits par blocs de 32 (MatMulNBits
d'onnxruntime, calcul en int8 : `accuracy_level` 4) ; le décodeur et le joiner
restent tels quels. Mesuré sur 14 leçons italiennes : 2,7 % de mots faux, comme
l'original, contre 3,5 % pour l'int8 publié par sherpa-onnx (quantification
dynamique, qui tronque des mots : « ingle » pour « inglese »), à la même vitesse.

  python3 -m venv /tmp/pk && /tmp/pk/bin/pip install onnx onnx_ir onnxruntime
  /tmp/pk/bin/python tools/build_parakeet.py <dossier de travail>

Le dossier reçoit parakeet-tdt-0.6b-v3-q8.tar (≈ 1 Go, non compressé : les poids
n'y gagnent que 6 %), sa taille et son empreinte SHA-256 pour `models.rs`.
Publier (une fois, jamais « latest ») :
  gh release create parakeet-1 --repo LivingTwice/lumen --latest=false \\
     --title "Parakeet V3 pour Lumen" --notes "…" <dossier>/parakeet-tdt-0.6b-v3-q8.tar
"""
import hashlib, os, shutil, subprocess, sys, tarfile

# export ONNX de sherpa-onnx, révision figée
REPO = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3/resolve/1a468a35cbba69418f126de829e75261dea4a4e4"
FILES = ["encoder.onnx", "encoder.weights", "decoder.onnx", "joiner.onnx", "tokens.txt"]
NAME = "parakeet-tdt-0.6b-v3-q8"

README = """Parakeet TDT 0.6B v3, pour Lumen

Modèle d'origine : nvidia/parakeet-tdt-0.6b-v3 (NVIDIA), sous licence
Creative Commons Attribution 4.0 (CC BY 4.0) :
https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
https://creativecommons.org/licenses/by/4.0/

Export ONNX : sherpa-onnx (k2-fsa), csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3.

Modification : les poids des produits de matrices de l'encodeur sont compressés
en 8 bits par blocs de 32 (MatMulNBits d'onnxruntime) ; décodeur et joiner
inchangés. Script : tools/build_parakeet.py du dépôt
https://github.com/LivingTwice/lumen
"""


def fetch(work):
    src = os.path.join(work, "fp32")
    os.makedirs(src, exist_ok=True)
    for f in FILES:
        dest = os.path.join(src, f)
        if not os.path.exists(dest):
            print("téléchargement", f, flush=True)
            subprocess.run(["curl", "-fL", "-C", "-", "-o", dest + ".part", f"{REPO}/{f}"], check=True)
            os.rename(dest + ".part", dest)
    return src


def quantize(src, out):
    import onnx
    from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer

    model = onnx.load(os.path.join(src, "encoder.onnx"), load_external_data=True)
    cfg = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, accuracy_level=4, bits=8)
    q = MatMulNBitsQuantizer(model, algo_config=cfg)
    q.process()
    onnx.save_model(q.model.model, os.path.join(out, "encoder.onnx"), save_as_external_data=False)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    work = os.path.abspath(sys.argv[1])
    src = fetch(work)
    out = os.path.join(work, NAME)
    shutil.rmtree(out, ignore_errors=True)
    os.makedirs(out)
    print("compression de l'encodeur…", flush=True)
    quantize(src, out)
    for f in ["decoder.onnx", "joiner.onnx", "tokens.txt"]:
        shutil.copy(os.path.join(src, f), out)
    with open(os.path.join(out, "LISEZMOI.txt"), "w", encoding="utf-8") as f:
        f.write(README)
    archive = os.path.join(work, NAME + ".tar")
    with tarfile.open(archive, "w", format=tarfile.GNU_FORMAT) as t:
        for f in sorted(os.listdir(out)):
            info = t.gettarinfo(os.path.join(out, f), arcname=f"{NAME}/{f}")
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = 0
            with open(os.path.join(out, f), "rb") as data:
                t.addfile(info, data)
    print(f"{archive}\ntaille : {os.path.getsize(archive)} octets\nsha256 : {sha256(archive)}")


if __name__ == "__main__":
    main()
