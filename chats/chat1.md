# Novel reading mobile app design

_Started 2026-10-01 07:18 UTC_

---

## User

Design a mobile app for Novel system :
1. Login/Register Page ( google / Facebook / icloud , email )
2. Home page :
- Show the most Popular novel By slide and title with short description
- Show 4 collection novel with slider
3. Detail page :
- Novel info 
- Chapter : When click will show popup to select language novel ( it will save to default in storage memory of device , when click other chapter it autoload this )
- Section related ( category ) and section the same as author
4. Category/Author page :
- Show list novel from category or author ( it can be filter )
- This page apply search
5. Reading page :
- Load by default novel language
- Tool icon when click will show tool for reading:
 + Reading Group : Fontsize , Background , Spacing , Gap , ...
 + AI Tool : Dictionary , AI Translate ( need have style translate , checkbox apply dictionary )
6. Profile Page:
- Need have info , login/logout
- History reading/Favorite Novel
7.Layout :
- Just reading don't apply 
- Search input using throttle to show list novel by search keyword input
- Menu can be a bottom icon group to have profile , library ( Favorite Novel ) , Category Page , Continue last chapter

## Assistant

_[tool: list_files]_

_[tool: read_file]_

_[tool: read_file]_

_[tool: dc_write]_

_[tool: dc_write]_

_[tool: dc_js_str_replace]_

_[tool: ready_for_verification]_

I've built **Inkleaf Novel App** as one interactive phone prototype using the Organic design system. Buttons on the left of the phone jump straight to any screen. It's still being checked in the background.

- **Login**: Google, Facebook and iCloud buttons, plus email login and register with field checks. "Skip" lets you browse as a guest.
- **Home**: a popular-novels slider you can swipe or let auto-advance, four collection rows, and a search box that updates results as you type.
- **Detail**: novel info, a favourite button, chapter list, and rows for the same category and the same author.
- **Language choice**: the first chapter you open asks which language to read in. The pick is saved on the device, so later chapters open straight in it. You can change it from the detail page, the reader, or Profile.
- **Category / Author**: search, filters for genre and status (ongoing or completed), and sorting.
- **Reader**: no bottom menu. The tools button opens a panel with two tabs:
  - **Reading**: font size, background, line spacing, paragraph gap, margins and alignment.
  - **AI tools**: dictionary lookup, the novel's name glossary, and translation with a choice of style and target language. A checkbox controls whether the glossary names are applied.
- **Profile and Library**: user info, log in / log out, reading history with Resume, and favourites.
- **Bottom menu**: Home, Category, a raised Continue button that opens your last chapter, Library and Profile.

A few things to know:
- Covers are placeholders showing just the title. Swap in real cover art when you have it.
- Translation and dictionary results are fake. Only English, Vietnamese and Spanish have sample text, and every novel shows the same sample chapter.
- The search delay, slider autoplay and starting screen can be changed in Tweaks.

## User

help me choose font best support for vietnamse ( can be using mono ) , remove some icon circle in background

## Assistant

_[tool: run_script]_

_[tool: dc_js_str_replace]_

_[tool: dc_set_props]_

_[tool: ready_for_verification]_

I recommend **Be Vietnam Pro**. It was designed for Vietnamese, so stacked accents like ữ, ặ and ỗ stay clean and readable even in the reader's longer text. It's now the default for the whole app. A background check is still running.

The old fonts (Caprasimo and Figtree) don't fully support Vietnamese accents, so I replaced them inside the phone. You can compare three options with the buttons beside the phone or in Tweaks:
- **Be Vietnam Pro**: the default.
- **Nunito**: rounded, closest to the soft Organic look.
- **JetBrains Mono**: the mono option you asked about.

I took out the decorative background circles on the login screen, home slider, detail header and profile card, and the ring on the covers. The small circles behind icons and buttons are still there. Tell me if you meant those instead.

## User

Change name to BenNovel

## Assistant

_[tool: run_script]_

_[tool: delete_file]_

_[tool: set_project_title]_

_[tool: ready_for_verification]_

The app is now called **BenNovel**, and the file is renamed to `BenNovel App.dc.html`. It opens with no errors.

I couldn't rename the project itself, because it already has a name ("Inkleaf novel app design"). You can change that yourself.

Because the app's saved-language setting was renamed too, any language you picked before is gone. The first chapter you open will ask again.

## User

Login need in 1 screen , don't shoud be scroll , let change social login to be 1 line

## Assistant

_[tool: read_file]_

_[tool: run_script]_

_[tool: show_html]_

_[tool: ready_for_verification]_

The login is now one fixed screen with no scrolling. Google, Facebook and iCloud sit side by side as three short buttons in one row.

To make it fit, I made the headline smaller, tightened the spacing and slightly reduced the input height. The terms line now sits at the bottom of the screen. I sized it so Register mode, which adds the name field, should still fit. A background check is still running.

## User

Found issues — fixing…

## Assistant

_[tool: run_script]_

_[tool: ready_for_verification]_

The background check found "Facebook" running into the right edge of its button, so I made the three social buttons more compact: tighter gaps, a smaller icon circle and 12px text. If a label is still too wide, for example with the wider mono font, it now cuts off with "…" instead of hitting the edge. A new check is running.

