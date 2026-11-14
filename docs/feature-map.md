# Feature map
Every user interaction in Drift, the test that drives it, and what the test checks. The tests live in `crates/drift-cli/tests/e2e/`. Each one starts the real `drift` binary on a pseudo-terminal against a fixture system, waits on what is drawn, and checks the files on disk after a write. `the_feature_map_matches_the_tests` keeps this page, the key map in [input-map.md](input-map.md) and the test files in step.
The harness is described in [testing.md](testing.md). An entry marked `planned` has no test yet.

## Everywhere

| Id | Interaction | Tests |
|---|---|---|
| `global.help` | `?` opens the help for the current screen; `?`, `q` or Esc closes it | planned |
| `global.palette` | `:` opens the command palette; typing narrows it, Enter runs the choice, Esc closes it, and a click on `[:] Palette` opens it | planned |
| `global.undo-last` | `u` opens the undo plan for the newest transaction from the tree, the diff and the history | planned |

## Tree

| Id | Interaction | Tests |
|---|---|---|
| `tree.quit` | `q` or a click on `[Q] Quit` leaves with exit code 0 | `tree::q_quits_and_exits_cleanly`, `tree::clicking_the_quit_hint_quits` |
| `tree.rescan` | `r` scans again and keeps the list, the selection and the marks while it runs | planned |
| `tree.down` | `j` and Down move the selection down | `tree::j_and_down_move_the_selection_down` |
| `tree.up` | `k` and Up move the selection up | `tree::k_and_up_move_the_selection_up` |
| `tree.collapse` | `h` and Left collapse a folder, or go from a file to its folder; a click on the caret collapses | `tree::h_and_left_collapse_a_folder_and_l_and_right_expand_it`, `tree::h_on_a_file_goes_to_its_folder_and_l_on_an_open_folder_goes_to_its_first_child`, `tree::clicking_a_row_selects_it_and_clicking_the_caret_toggles_the_folder` |
| `tree.expand` | `l` and Right expand a folder, or go to its first child | `tree::h_and_left_collapse_a_folder_and_l_and_right_expand_it`, `tree::h_on_a_file_goes_to_its_folder_and_l_on_an_open_folder_goes_to_its_first_child` |
| `tree.top` | `g` and Home jump to the first row | `tree::g_and_home_jump_to_the_first_row_and_capital_g_and_end_to_the_last` |
| `tree.bottom` | `G` and End jump to the last row | `tree::g_and_home_jump_to_the_first_row_and_capital_g_and_end_to_the_last` |
| `tree.page-down` | PageDown moves by a screenful | `tree::page_down_and_page_up_move_by_a_screenful` |
| `tree.page-up` | PageUp moves back by a screenful | `tree::page_down_and_page_up_move_by_a_screenful` |
| `tree.open` | Enter opens a file's diff and toggles a folder; a click on a row selects it | `tree::enter_opens_the_diff_of_a_file_and_toggles_a_folder`, `tree::clicking_a_row_selects_it_and_clicking_the_caret_toggles_the_folder` |
| `tree.search` | `/` opens a search that narrows the tree as you type, Backspace widens it, Enter keeps it, Esc clears it, and a no-match search shows 0 files | `filter::slash_opens_a_search_that_narrows_the_tree_as_you_type`, `filter::backspace_widens_the_search_again`, `filter::enter_keeps_the_search_and_leaves_the_input_and_escape_clears_it`, `filter::a_search_with_no_match_says_so_and_escape_recovers`, `filter::clicking_the_search_hint_opens_the_search` |
| `tree.filter-unchanged` | `1` shows only unchanged files | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.filter-upstream` | `2` shows only files with upstream changes | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.filter-override` | `3` shows only local overrides | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.filter-conflict` | `4` shows only conflicts; a click on the count does the same and a second click clears it | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter`, `filter::clicking_a_status_count_filters_and_clicking_it_again_clears` |
| `tree.filter-user-only` | `5` shows only user-only files | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.filter-missing` | `6` shows only missing files | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.clear-filter` | `0` clears the status filter | `filter::the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter` |
| `tree.mark` | Space marks or unmarks the row (a folder marks everything under it, shown as `[-]` when partly marked); a click on the checkbox marks that file | `filter::space_marks_the_selected_folder_and_everything_under_it`, `filter::clicking_a_checkbox_marks_that_row` |
| `tree.mark-all` | `a` marks every file in view, which a filter narrows | `filter::a_marks_everything_in_view_and_escape_clears_the_marks`, `filter::a_marks_only_what_the_filter_shows` |
| `tree.back` | Esc clears the search, then the filter, then the marks | `filter::escape_clears_the_search_the_filter_and_the_marks_one_after_another`, `filter::a_marks_everything_in_view_and_escape_clears_the_marks` |
| `tree.keep` | `o` opens the keep-mine plan for the marked files or the selected one | planned |
| `tree.take` | `t` opens the take-upstream plan | planned |
| `tree.merge` | `m` opens the merge plan for a file that merges cleanly | planned |
| `tree.ignore` | `i` ignores the selected file | planned |
| `tree.history` | `H` opens the history | planned |

## File states

| Id | Interaction | Tests |
|---|---|---|
| `state.unchanged` | an unchanged file is listed with `✓ unchanged`, and the action keys decline with a reason | planned |
| `state.upstream` | an upstream-changed file is listed with `↓ upstream changed`, and take or merge applies it | planned |
| `state.override` | a local override is listed with `≠ local override`, and keep records it as reviewed | planned |
| `state.conflict` | a conflict is listed with `✗ conflict` and opens the four-pane diff | planned |
| `state.user-only` | a file with no template is listed with `+ user-only` and is never written | planned |
| `state.missing` | a template with no file is listed with `? missing`, and take creates the file | planned |
| `badge.merges-cleanly` | `[merges cleanly]` marks a conflict whose edits touch different lines, and merge writes it | planned |
| `badge.base-unknown` | `[base unknown]` marks a file whose reviewed base is missing, and the diff says so | planned |
| `badge.deleted-by-you` | `[deleted by you]` marks a file you removed that upstream changed | planned |

## Flows

| Id | Interaction | Tests |
|---|---|---|
| `flow.take` | take upstream from the tree: the plan lists the file, confirm writes it, the screen says what was done, the list refreshes, and the file on disk equals the template | planned |
| `flow.keep` | keep mine from the tree: nothing is written to the file, the base is recorded as reviewed, and the row reads as an override | planned |
| `flow.merge` | merge from the tree writes the merged text and a backup of the original | planned |
| `flow.both` | `b` in the diff keeps both sides of a hunk and the written file has both | planned |
| `flow.edit` | `e` hands a hunk to `$EDITOR`, and what the editor saves becomes the result | planned |
| `flow.write` | `w` in the diff opens the plan for the result and confirm writes it | planned |
| `flow.confirm` | Confirm writes exactly the files listed, creates a transaction with backups and leaves the live files as planned | planned |
| `flow.cancel` | Cancel, `n`, `q` and Esc close the plan and nothing on disk changes | planned |
| `flow.plan-preview` | the plan for an existing file names its action and backup path, and a new file reads `new file` | planned |
| `flow.plan-keys` | Tab, arrows and `h`/`l` move between the buttons, `j`/`k` select a file, `d` expands its diff, and a click presses a button | planned |
| `refusal.update-lock` | a held Omarchy update lock refuses the write with a message, and nothing changes | planned |
| `refusal.stale-lock` | a lock file that exists but is not held does not refuse a write | planned |
| `refusal.pacman-lock` | pacman's database lock refuses the write | planned |
| `refusal.changed-since-plan` | a file changed after the plan was made is refused with a stale-hash message | planned |
| `reload.keeps-list` | a reload after a write or a rescan never empties or flashes the list, and keeps the selection | planned |
| `modal.borders` | a dialog is drawn with solid borders | planned |

## History

| Id | Interaction | Tests |
|---|---|---|
| `history.back` | `q` or Esc returns to the tree | planned |
| `history.refresh` | `r` reloads the list | planned |
| `history.undo` | Enter opens the undo plan for the selected transaction and confirm restores the files | planned |
| `history.undo-force` | `U` undoes and also restores files changed since | planned |
| `history.down` | `j` and Down move down | planned |
| `history.up` | `k` and Up move up | planned |
| `history.top` | `g` and Home select the newest | planned |
| `history.bottom` | `G` and End select the oldest | planned |
| `history.page-down` | PageDown moves by a screenful | planned |
| `history.page-up` | PageUp moves back by a screenful | planned |

## Diff

| Id | Interaction | Tests |
|---|---|---|
| `diff.quit` | `q` leaves the diff for the tree | planned |
| `diff.next` | `n` selects the next hunk | planned |
| `diff.previous` | `p` selects the previous hunk | planned |
| `diff.next-conflict` | `N` jumps to the next unresolved conflict and says when there is none | planned |
| `diff.previous-conflict` | `P` jumps to the previous unresolved conflict | planned |
| `diff.layout` | `v` cycles the layouts | planned |
| `diff.focus-next` | Tab moves the focus to the next pane | planned |
| `diff.focus-previous` | Shift+Tab moves it back | planned |
| `diff.widen` | `]` widens the focused pane | planned |
| `diff.narrow` | `[` narrows it | planned |
| `diff.down` | Down scrolls the panes together | planned |
| `diff.up` | Up scrolls back | planned |
| `diff.mine` | `o` takes my side for the hunk | planned |
| `diff.theirs` | `t` takes upstream's side for the hunk | planned |
| `diff.both` | `b` takes both sides | planned |
| `diff.edit` | `e` edits the hunk | planned |
| `diff.write` | `w` writes the result | planned |

## Plan dialog

| Id | Interaction | Tests |
|---|---|---|
| `plan.confirm` | `y` or Enter on Confirm writes | planned |
| `plan.cancel` | `n`, `q`, Esc or Enter on Cancel writes nothing | planned |
| `plan.focus` | Tab, Left, Right, `h` and `l` move between the buttons | planned |
| `plan.select` | `j`, `k`, arrows, Home and End select a file | planned |
| `plan.expand` | `d` expands or collapses the selected file's diff | planned |

## Window and theme

| Id | Interaction | Tests |
|---|---|---|
| `window.resize` | resizing keeps the screen, the selection and an open dialog and repaints at the new size | planned |
| `window.layouts` | the diff shows five columns, two panes or one column by width | planned |
| `theme.dark` | the dark theme draws legible status colours | planned |
| `theme.light` | the light theme drawn from the Omarchy colours file is legible | planned |

## Command line

| Id | Interaction | Tests |
|---|---|---|
| `widget.install` | `drift widget install` copies the plugin, enables it and places it on the bar | planned |
| `widget.sync` | `drift widget sync` refreshes the copy and leaves local edits unless forced | planned |
| `widget.remove` | `drift widget remove` disables the widget and deletes the copy | planned |
| `notify.send` | `drift notify` sends one notice when files drifted | planned |
| `notify.skip` | `drift notify` sends nothing when nothing drifted, when an update runs or when notices are off | planned |
| `notify.switch` | `drift notify on` and `off` keep the choice | planned |
