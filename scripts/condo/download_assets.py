"""Download the selected CC0 furniture, retaining provenance and checksums."""
import hashlib
import io
import json
from pathlib import Path
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2] / "scenes/toronto_condo/models"
ASSETS = ["modern_arm_chair_01", "potted_plant_02"]


def request(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "TorontoCondoScene/1.0"}), timeout=60).read()


def fetch_file(folder, relative, source):
    path = folder / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    content = path.read_bytes() if path.exists() else request(source["url"])
    if hashlib.md5(content).hexdigest() != source["md5"]:
        raise ValueError(f"Asset checksum mismatch: {relative}")
    path.write_bytes(content)
    return {"path": relative, "url": source["url"], "sha256": hashlib.sha256(content).hexdigest()}


def download(asset):
    folder = ROOT / asset
    folder.mkdir(parents=True, exist_ok=True)
    metadata = json.loads(request(f"https://api.polyhaven.com/files/{asset}"))
    info = json.loads(request(f"https://api.polyhaven.com/info/{asset}"))
    model = metadata["gltf"]["2k"]["gltf"]
    files = {f"{asset}_2k.gltf": model, **model["include"]}
    records = []
    for relative, source in files.items():
        records.append(fetch_file(folder, relative, source))
    credit = {"id": asset, "name": info["name"], "authors": info["authors"],
              "source": f"https://polyhaven.com/a/{asset}", "license": "CC0-1.0",
              "license_url": "https://polyhaven.com/license", "files": records}
    (folder / "provenance.json").write_text(json.dumps(credit, indent=2) + "\n")
    print(asset, len(records), "files downloaded and checked", flush=True)


def download_surface(asset):
    folder = ROOT / asset
    metadata = json.loads(request(f'https://api.polyhaven.com/files/{asset}'))
    info = json.loads(request(f'https://api.polyhaven.com/info/{asset}'))
    records = []
    for channel in ['Diffuse','nor_gl','Rough']:
        source = metadata[channel]['2k']['jpg']
        records.append(fetch_file(folder, source['url'].rsplit('/',1)[1], source))
    credit = {'id':asset,'name':info['name'],'authors':info['authors'], 'dimensions_mm':info['dimensions'],
        'source':f'https://polyhaven.com/a/{asset}','license':'CC0-1.0','license_url':'https://polyhaven.com/license','files':records}
    (folder/'provenance.json').write_text(json.dumps(credit,indent=2)+'\n')
    print(asset,'downloaded and checked',flush=True)


def download_stone():
    asset = 'Travertine009'
    folder = ROOT / asset
    folder.mkdir(parents=True, exist_ok=True)
    metadata = request(f'https://ambientcg.com/api/v2/full_json?id={asset}&include=downloadData,displayData,dimensions')
    info = json.loads(metadata)['foundAssets'][0]
    downloads = info['downloadFolders']['default']['downloadFiletypeCategories']['zip']['downloads']
    item = next(item for item in downloads if item['attribute'] == '2K-PNG')
    archive = request(item['downloadLink'])
    digest = hashlib.sha256(archive).hexdigest()
    if digest != '838d56d2abac7b7451b54c756afcd36244b55f3d71a968d674b1e9f8ac4fe4b3':
        raise ValueError('Travertine009 archive changed; review the new source before updating the scene')
    records = []
    with zipfile.ZipFile(io.BytesIO(archive)) as zipped:
        for name in zipped.namelist():
            if Path(name).name != name or not name.endswith('.png'):
                continue
            content = zipped.read(name)
            (folder / name).write_bytes(content)
            records.append({'path': name, 'bytes': len(content), 'sha256': hashlib.sha256(content).hexdigest()})
    credit = {'id': asset, 'source': info['shortLink'], 'credit': 'ambientCG / Lennart Demes',
        'license': 'CC0-1.0', 'license_url': 'https://docs.ambientcg.com/license/',
        'dimensions_m': [1.2, 1.2], 'source_technique': info['creationMethod'],
        'download_url': item['downloadLink'], 'archive_bytes': len(archive),
        'archive_sha256': digest, 'files': records}
    (folder / 'metadata.json').write_bytes(metadata)
    (folder / 'provenance.json').write_text(json.dumps(credit, indent=2) + '\n')
    print(asset, 'downloaded and checked', flush=True)


if __name__ == "__main__":
    for asset in ASSETS:
        download(asset)
    download_surface('wood_floor')
    download_surface('terlenka')
    download_stone()
