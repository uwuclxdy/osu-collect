# Importing into osu!

Every download produces the `.osz` files and a ready-to-import `collection.db`, so you get an actual osu! collection.

## Where the files land

Everything goes into the download directory you picked on the get maps tab, in a subfolder per run: a whole collection gets `<name>-<id>`; a `find` run gets `search-`/`filter-` plus its search; `update` runs (plus a part-picked collection) get `update-<id>` or `update-<n>-collections`. Focus the download directory field; the form shows the exact path before you start. The generated `collection.db` sits in that folder, next to the `.osz` files. That folder is what you point lazer's import at.

## osu! lazer

1. Import all downloaded maps into lazer.
2. Click `Run first time setup`, then `Next` until the **Import screen**.
3. Set `previous osu! install` to the **folder of the collection** you downloaded.
4. Click `Import content from previous version`.
5. Both the maps and the collection are imported.

Already past first-time setup? **Settings → General → `Run setup wizard`** brings the import screen back.

## osu! stable

Drag the downloaded `.osz` files into osu!, then merge the generated `collection.db` with a tool like [Collection Manager](https://github.com/Piotrekol/CollectionManager). If you have no existing collections, you can just replace your `collection.db` with the generated one.

Your own `collection.db` lives in the osu! install folder; back it up before merging.
