# The museum's paintings

Paintings in the Rijksmuseum, Amsterdam, whose images the museum releases
into the public domain (https://creativecommons.org/publicdomain/mark/1.0/;
https://www.rijksmuseum.nl/en/research/conduct-research/data/policy),
fetched through its IIIF image server fitted within 512 pixels by
`deploy/paintings.sh` from `paintings.tsv`. `build.rs` packs them as
compressed texture layers that the underwater museum streams in as it comes
near and drops once it is left behind. The natural-history hall hangs the
first group, the picture gallery the grand masters.

| File | Hall | Painting | Painter | Object |
|---|---|---|---|---|
| coorte-shells-on-a-stone-plinth.jpg | natural history | Shells on a Stone Plinth | Adriaen Coorte | https://id.rijksmuseum.nl/200614287 |
| coorte-still-life-with-asparagus.jpg | natural history | Still Life with Asparagus | Adriaen Coorte | https://id.rijksmuseum.nl/200109193 |
| coorte-a-bowl-of-strawberries-on-a-stone.jpg | natural history | A Bowl of Strawberries on a Stone Plinth | Adriaen Coorte | https://id.rijksmuseum.nl/200367359 |
| d-hondecoeter-a-pelican-and-other-birds.jpg | natural history | A Pelican and other Birds near a Pool, Known as ‘The Floating Feather’ | Melchior d'Hondecoeter | https://id.rijksmuseum.nl/200109383 |
| d-hondecoeter-the-menagerie.jpg | natural history | The Menagerie | Melchior d'Hondecoeter | https://id.rijksmuseum.nl/200109385 |
| d-hondecoeter-ducks.jpg | natural history | Ducks | Melchior d'Hondecoeter | https://id.rijksmuseum.nl/200108439 |
| d-hondecoeter-a-hunter-s-bag-near-a-tree.jpg | natural history | A Hunter’s Bag near a Tree Stump with a Magpie, Known as ‘The Contemplative Magpie’ | Melchior d'Hondecoeter | https://id.rijksmuseum.nl/200109384 |
| savery-landscape-with-ruins-cattle-and-d.jpg | natural history | Landscape with ruins, cattle and deer | Roelant Savery | https://id.rijksmuseum.nl/20026269 |
| savery-wild-boar-hunt-in-a-rocky-landsca.jpg | natural history | Wild boar hunt in a rocky landscape | Roelant Savery | https://id.rijksmuseum.nl/20026271 |
| asselijn-the-threatened-swan.jpg | natural history | The Threatened Swan | Jan Asselijn | https://id.rijksmuseum.nl/200108146 |
| velde-a-ship-on-the-high-seas-caught-by.jpg | natural history | A Ship on the High Seas Caught by a Squall, Known as ‘The Gust’ | Willem van de Velde | https://id.rijksmuseum.nl/200108359 |
| velde-the-cannon-shot.jpg | natural history | The Cannon Shot | Willem van de Velde | https://id.rijksmuseum.nl/200108360 |
| velde-ships-near-the-coast-during-a-calm.jpg | natural history | Ships near the Coast during a Calm | Willem van de Velde | https://id.rijksmuseum.nl/20026835 |
| ruysch-still-life-with-flowers-in-a-glas.jpg | natural history | Still Life with Flowers in a Glass Vase | Rachel Ruysch | https://id.rijksmuseum.nl/200108485 |
| post-brazilian-landscape.jpg | natural history | Brazilian Landscape | Frans Post | https://id.rijksmuseum.nl/200107886 |
| kessel-sprig-of-white-currants-with-inse.jpg | natural history | Sprig of White Currants with Insects | Jan van Kessel | https://id.rijksmuseum.nl/20013605 |
| potter-orpheus-and-the-animals.jpg | natural history | Orpheus and the Animals | Paulus Potter | https://id.rijksmuseum.nl/200107888 |
| cuyp-river-landscape-with-cows.jpg | natural history | River Landscape with Cows | Aelbert Cuyp | https://id.rijksmuseum.nl/20027732 |
| heem-festoon-of-fruit-and-flowers.jpg | natural history | Festoon of Fruit and Flowers | Jan Davidsz. de Heem | https://id.rijksmuseum.nl/200108431 |
| rembrandt-the-night-watch-militia-company-of.jpg | grand masters | The Night Watch: Militia Company of District II under the Command of Captain Frans Banninck Cocq | Rembrandt van Rijn | https://id.rijksmuseum.nl/200107928 |
| rembrandt-the-sampling-officials-of-the-amste.jpg | grand masters | The Sampling Officials of the Amsterdam Drapers’ Guild, Known as ‘The Syndics’ | Rembrandt van Rijn | https://id.rijksmuseum.nl/200107929 |
| vermeer-the-milkmaid.jpg | grand masters | The Milkmaid | Johannes Vermeer | https://id.rijksmuseum.nl/200108369 |
| vermeer-view-of-houses-in-delft-known-as.jpg | grand masters | View of Houses in Delft, Known as ‘The Little Street’ | Johannes Vermeer | https://id.rijksmuseum.nl/200108371 |
| vermeer-woman-reading-a-letter.jpg | grand masters | Woman Reading a Letter | Johannes Vermeer | https://id.rijksmuseum.nl/200108372 |
| vermeer-the-love-letter.jpg | grand masters | The Love Letter | Johannes Vermeer | https://id.rijksmuseum.nl/200108370 |
| hals-the-merry-drinker.jpg | grand masters | The Merry Drinker | Frans Hals | https://id.rijksmuseum.nl/200109347 |
| hals-portrait-of-a-married-couple-likely.jpg | grand masters | Portrait of a Married Couple, Likely Isaac Abrahamsz Massa and Beatrix van der Laen | Frans Hals | https://id.rijksmuseum.nl/200109346 |
| steen-the-merry-family.jpg | grand masters | The Merry Family | Jan Steen | https://id.rijksmuseum.nl/200108012 |
| steen-children-teaching-a-cat-to-dance-k.jpg | grand masters | Children Teaching a Cat to Dance, Known as The Dancing Lesson | Jan Steen | https://id.rijksmuseum.nl/200107997 |
| ruisdael-the-windmill-at-wijk-bij-duurst.jpg | grand masters | The Windmill at Wijk bij Duurstede | Jacob van Ruisdael | https://id.rijksmuseum.nl/200107959 |
| ruisdael-view-of-haarlem-from-the-northw.jpg | grand masters | View of Haarlem from the Northwest, with the Bleaching Fields in the Foreground | Jacob van Ruisdael | https://id.rijksmuseum.nl/200107958 |
| hooch-a-maid-with-a-child-in-a-pantry.jpg | grand masters | A Maid with a Child in a Pantry | Pieter de Hooch | https://id.rijksmuseum.nl/200109391 |
| borch-helena-van-der-schalcke.jpg | grand masters | Helena van der Schalcke | Gerard ter Borch | https://id.rijksmuseum.nl/200108271 |
| gogh-self-portrait.jpg | grand masters | Self-portrait | Vincent van Gogh | https://id.rijksmuseum.nl/200109794 |
| hobbema-a-watermill.jpg | grand masters | A Watermill | Meindert Hobbema | https://id.rijksmuseum.nl/20015940 |
| fabritius-portrait-of-abraham-de-potter.jpg | grand masters | Portrait of Abraham de Potter, Amsterdam Silk Merchant | Carel Fabritius | https://id.rijksmuseum.nl/20027868 |
| leyster-the-serenade.jpg | grand masters | The Serenade | Judith Leyster | https://id.rijksmuseum.nl/200109431 |
| goya-portrait-of-don-ram-n-satu.jpg | grand masters | Portrait of Don Ramón Satué | Francisco de Goya | https://id.rijksmuseum.nl/200109334 |
| crivelli-mary-magdalene.jpg | grand masters | Mary Magdalene | Carlo Crivelli | https://id.rijksmuseum.nl/200109206 |
| saenredam-interior-of-the-church-of-st-b.jpg | grand masters | Interior of the Church of St Bavo in Haarlem | Pieter Saenredam | https://id.rijksmuseum.nl/200107963 |
| maes-old-woman-saying-grace-known-as-the.jpg | grand masters | Old Woman Saying Grace, Known as ‘The Prayer without End’ | Nicolaes Maes | https://id.rijksmuseum.nl/20015947 |
| rembrandt-man-in-oriental-dress.jpg | grand masters | Man in Oriental Dress | Rembrandt van Rijn | https://id.rijksmuseum.nl/200107930 |
