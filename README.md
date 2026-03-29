# clippings2mbox

Converts a Kindle [`My Clippings.txt`][clippings] file into an [mbox]-format
mailbox, one message per highlight, for import into DEVONthink or any
mbox-compatible mail client.

## Usage

[Pull your `My Clippings.txt` file off your Kindle][clippings], then run:

```sh-session
$ cargo run -- <clippings.txt> <output.mbox>
```

Rerunning with a subsequent version of the clippings file will add only new
clippings; duplicates will be skipped.

## Features

- Clippings have deterministic message IDs. DEVONthink will skip message IDs
  it's already imported, so the mailbox is safe to re-import as it grows over
  time.

- Adjacent and overlapping highlights from the same book are merged into the
  longest contiguous passage.

## Limitations

- No support for Kindle notes, which will be skipped with a warning.

## Message format

Each highlight becomes a message with:

- **From:** author's name (or the document's title)
- **Subject:** citation and page
- **Date:** when the highlight was made
- **Body:** the highlighted text

```
From kindle@clippings2mbox Thu Sep 18 20:46:43 2025
From: =?UTF-8?Q?Karl_Ove_Knausgaard?= <kindle@clippings2mbox>
Date: Thu, 18 Sep 2025 20:46:43 +0000
Subject: =?UTF-8?Q?Knausgaard,_"The_Morning_Star",_309?=
Message-ID: <9af5bd567f2bb98d157aa10aac09d5931401e6b0@clippings2mbox>
X-Kindle-Location-Start: 5115
X-Kindle-Location-End: 5117
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8
Status: O
Content-Length: 199
Lines: 1

I thought to myself, or pictured in my mind—for thoughts come as both pictures and words, a bit like light, which comes as both particles and waves, one could imagine, as indeed I had, many times.
```

[clippings]: https://docs.clippings.io/importing/importing-your-myclippingstxt-file/
[mbox]: https://en.wikipedia.org/wiki/Mbox
