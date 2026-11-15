# Feature map
Every user interaction in Drift, the test that drives it, and what the test checks. The tests live in `crates/drift-cli/tests/e2e/`. Each one starts the real `drift` binary on a pseudo-terminal against a fixture system, waits on what is drawn, and checks the files on disk after a write. `the_feature_map_matches_the_tests` keeps this page, the key map in [input-map.md](input-map.md) and the test files in step.
The harness is described in [testing.md](testing.md). An entry marked `planned` has no test yet, and the behaviour may not exist yet either.

## Everywhere

| Id | Interaction | Tests |
|---|---|---|
| `global.help` | `?` opens the help for the current screen; `?`, `q` or Esc closes it | `overlay::question_mark_opens_the_help_with_every_section_and_escape_closes_it`, `overlay::clicking_the_help_hint_opens_the_help`, `overlay::the_help_scrolls_with_j_and_k_through_every_section`, `overlay::the_help_lists_every_action_of_the_key_map_document` |
| `global.palette` | `:` opens the command palette; typing narrows it, Enter runs the choice, Esc closes it, and a click on `[:] Palette` opens it | `overlay::colon_opens_the_palette_listing_the_commands_and_escape_closes_it`, `overlay::clicking_the_palette_hint_opens_it`, `overlay::typing_narrows_the_palette_and_backspace_widens_it`, `overlay::enter_runs_the_first_match_and_closes_the_palette`, `overlay::down_chooses_the_next_match_before_enter`, `overlay::clicking_a_palette_row_runs_that_command`, `overlay::a_palette_search_with_no_match_runs_nothing_and_stays_open`, `overlay::a_command_with_no_key_of_its_own_is_reachable_from_the_palette` |
| `global.undo-last` | `u` in the tree opens the undo plan for the newest transaction (covered from the tree only) | `plan::u_in_the_tree_opens_the_undo_plan_for_the_newest_transaction` |

## Tree

| Id | Interaction | Tests |
|---|---|---|
| `tree.quit` | `q` or a click on `[Q] Quit` leaves with exit code 0 | `tree::q_quits_and_exits_cleanly`, `tree::clicking_the_quit_hint_quits` |
| `tree.rescan` | `r` scans again and keeps the list, the selection (by file, even when rows appear above it) and the marks, with no frame that drops rows | `tree::r_rescans_and_keeps_the_list_the_selection_and_the_marks` |
| `tree.down` | `j` and Down move the selection down | `tree::j_and_down_move_the_selection_down` |
| `tree.up` | `k` and Up move the selection up | `tree::k_and_up_move_the_selection_up` |
| `tree.collapse` | `h` and Left collapse a folder, or go from a file to its folder; a click on the caret collapses | `tree::h_and_left_collapse_a_folder_and_l_and_right_expand_it`, `tree::h_on_a_file_goes_to_its_folder_and_l_on_an_open_folder_goes_to_its_first_child`, `tree::clicking_a_row_selects_it_and_clicking_the_caret_toggles_the_folder` |
| `tree.expand` | `l` and Right expand a folder, or go to its first child | `tree::h_and_left_collapse_a_folder_and_l_and_right_expand_it`, `tree::h_on_a_file_goes_to_its_folder_and_l_on_an_open_folder_goes_to_its_first_child` |
| `tree.top` | `g` and Home jump to the first row | `tree::g_and_home_jump_to_the_first_row_and_capital_g_and_end_to_the_last` |
| `tree.bottom` | `G` and End jump to the last row | `tree::g_and_home_jump_to_the_first_row_and_capital_g_and_end_to_the_last` |
| `tree.page-down` | PageDown moves down by half the height of the tree pane | `tree::page_down_and_page_up_move_by_half_the_tree_pane` |
| `tree.page-up` | PageUp moves up by half the height of the tree pane | `tree::page_down_and_page_up_move_by_half_the_tree_pane` |
| `tree.open` | Enter, or a double-click, opens a file's diff and toggles a folder; a single click on a row selects it | `tree::enter_opens_the_diff_of_a_file_and_toggles_a_folder`, `tree::double_clicking_a_file_opens_its_diff`, `tree::clicking_a_row_selects_it_and_clicking_the_caret_toggles_the_folder` |
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
| `tree.back` | Esc clears the search, then the filter, then the marks, one per press | `filter::escape_clears_the_search_then_the_filter_then_the_marks`, `filter::a_marks_everything_in_view_and_escape_clears_the_marks` |
| `tree.keep` | `o` opens the keep-mine plan for the marked files or the selected one | `plan::o_keeps_mine_writes_nothing_and_records_the_template_as_reviewed` |
| `tree.take` | `t` opens the take-upstream plan | `plan::y_confirms_and_writes_the_file_a_backup_and_one_transaction`, `plan::a_plan_for_everything_writes_every_listed_file_and_creates_the_missing_one` |
| `tree.merge` | `m` opens the merge plan for a file that merges cleanly | `plan::m_merges_a_clean_merge_and_writes_the_combined_text_with_a_backup` |
| `tree.ignore` | `i` ignores the selected file or the marked files | `ignore::i_ignores_the_selected_file_and_records_it_in_the_state_folder`, `ignore::i_ignores_every_marked_file_and_not_the_selected_one`, `ignore::an_ignored_file_stays_out_of_the_list_in_the_next_session`, `ignore::the_palette_ignore_command_ignores_the_selected_file` |
| `tree.history` | `H` opens the history | `history::h_opens_the_history_listing_every_transaction_newest_first` |

## File states

| Id | Interaction | Tests |
|---|---|---|
| `state.unchanged` | an unchanged file is listed with `✓ unchanged`, and the action keys decline with a reason | planned |
| `state.upstream` | an upstream-changed file is listed with `↓ upstream changed`, and take or merge applies it | planned |
| `state.override` | a local override is listed with `≠ local override`, and keep records it as reviewed | planned |
| `state.conflict` | a conflict is listed with `✗ conflict` and opens the four-pane diff | planned |
| `state.user-only` | a file with no template is listed with `+ user-only` and is never written | planned |
| `state.missing` | a template with no file is listed with `? missing`, and take creates the file | planned |
| `badge.merges-cleanly` | `[merges cleanly]` marks an `upstream changed` file you also edited on different lines, and merge writes the combined text | planned |
| `badge.base-unknown` | `[base unknown]` marks a local override whose file differs from the template and has no recorded base (no reviewed, snapshot or package-cache base); the diff header has its own separate base-unknown note | planned |
| `badge.deleted-by-you` | `[deleted by you]` marks a missing file for which some base is known (reviewed, snapshot or cache); it does not need an upstream change | planned |

## Flows

| Id | Interaction | Tests |
|---|---|---|
| `flow.take` | take upstream from the tree: the plan lists the file, confirm writes it, the screen says what was done, the list refreshes, and the file on disk equals the template | `plan::y_confirms_and_writes_the_file_a_backup_and_one_transaction`, `plan::a_plan_for_everything_writes_every_listed_file_and_creates_the_missing_one`, `plan::after_a_confirmed_write_the_list_refreshes_in_place_and_keeps_the_selection` |
| `flow.keep` | keep mine from the tree: nothing is written to the file, the base is recorded as reviewed, and the row reads as an override | `plan::o_keeps_mine_writes_nothing_and_records_the_template_as_reviewed` |
| `flow.merge` | merge from the tree writes the merged text and a backup of the original | `plan::m_merges_a_clean_merge_and_writes_the_combined_text_with_a_backup` |
| `flow.both` | `b` in the diff keeps both sides of a hunk and the written file has both | `diff::b_keeps_both_sides_in_the_result_and_write_puts_both_on_disk` |
| `flow.edit` | `e` hands a hunk to `$EDITOR`, and what the editor saves becomes the result | `diff::e_hands_the_hunk_to_the_editor_and_write_puts_what_it_saved_on_disk` |
| `flow.write` | `w` in the diff opens the plan for the result and confirm writes it | `diff::t_takes_theirs_for_the_hunk_and_write_puts_it_on_disk_with_a_backup_of_the_original`, `diff::the_write_hint_opens_the_plan_for_the_result` |
| `flow.confirm` | Confirm writes exactly the files listed, creates a transaction with backups and leaves the live files as planned | `plan::y_confirms_and_writes_the_file_a_backup_and_one_transaction`, `plan::enter_on_the_confirm_button_writes_after_tab_moves_the_focus_to_it`, `plan::clicking_the_confirm_button_writes`, `plan::a_plan_for_everything_writes_every_listed_file_and_creates_the_missing_one` |
| `flow.cancel` | Cancel, `n`, `q` and Esc close the plan and nothing on disk changes | `plan::n_cancels_and_changes_nothing`, `plan::q_cancels_the_plan_instead_of_quitting_and_changes_nothing`, `plan::escape_cancels_and_changes_nothing`, `plan::enter_on_the_default_cancel_button_changes_nothing`, `plan::clicking_the_cancel_button_changes_nothing` |
| `flow.plan-preview` | the plan for an existing file names its action and backup path, and a new file reads `new file` | `plan::the_plan_for_an_existing_file_names_the_action_the_real_path_and_the_backup`, `plan::the_plan_for_a_missing_file_reads_new_file_and_has_no_backup` |
| `flow.plan-keys` | Tab, arrows and `h`/`l` move between the buttons, `j`/`k` select a file, `d` expands its diff, and a click presses a button | `plan::tab_arrows_h_and_l_move_between_the_buttons`, `plan::j_k_arrows_home_and_end_select_a_file_in_the_plan`, `plan::clicking_a_file_in_the_plan_selects_it`, `plan::d_expands_the_selected_files_diff_and_a_second_d_collapses_it`, `plan::clicking_the_confirm_button_writes`, `plan::clicking_the_cancel_button_changes_nothing` |
| `refusal.update-lock` | a held Omarchy update lock refuses the write with a message, and nothing changes | `plan::a_held_update_lock_refuses_the_write_and_changes_nothing` |
| `refusal.stale-lock` | a lock file that exists but is not held does not refuse a write | `plan::a_stale_update_lock_file_that_nobody_holds_does_not_refuse_the_write` |
| `refusal.pacman-lock` | pacman's database lock refuses the write | `plan::pacmans_database_lock_refuses_the_write_and_changes_nothing` |
| `refusal.changed-since-plan` | a file changed after the plan was made is refused with a stale-hash message | `plan::a_file_changed_after_the_plan_was_made_is_refused_and_left_as_it_is` |
| `reload.keeps-list` | a rescan never empties or flashes the list | `tree::r_rescans_and_keeps_the_list_the_selection_and_the_marks` |
| `reload.after-write` | the list after a confirmed write refreshes in place, keeps the selection and shows the written file as unchanged | `plan::after_a_confirmed_write_the_list_refreshes_in_place_and_keeps_the_selection` |
| `modal.borders` | a dialog is drawn with solid borders | planned |

## History

| Id | Interaction | Tests |
|---|---|---|
| `history.back` | `q` or Esc returns to the tree | `history::q_and_escape_return_to_the_tree` |
| `history.refresh` | `r` reloads the list | `history::r_reloads_the_list_and_shows_what_appeared_on_disk` |
| `history.undo` | Enter opens the undo plan for the selected transaction and confirm restores the files | `plan::enter_in_the_history_undoes_a_transaction_and_restores_the_saved_bytes` |
| `history.undo-force` | `U` undoes and also restores files changed since | `plan::undo_refuses_a_file_changed_since_and_capital_u_restores_it_anyway` |
| `history.down` | `j` and Down move down, and a click selects a row | `history::j_and_down_move_down_and_k_and_up_move_up`, `history::clicking_a_row_selects_it` |
| `history.up` | `k` and Up move up | `history::j_and_down_move_down_and_k_and_up_move_up` |
| `history.top` | `g` and Home select the newest | `history::g_and_home_select_the_newest_and_capital_g_and_end_the_oldest` |
| `history.bottom` | `G` and End select the oldest | `history::g_and_home_select_the_newest_and_capital_g_and_end_the_oldest` |
| `history.page-down` | PageDown moves down by half the height of the history pane | `history::page_down_and_page_up_move_by_half_the_history_pane` |
| `history.page-up` | PageUp moves up by half the height of the history pane | `history::page_down_and_page_up_move_by_half_the_history_pane` |

## Diff

| Id | Interaction | Tests |
|---|---|---|
| `diff.quit` | `q` leaves the diff for the tree | `diff::q_leaves_the_diff_for_the_tree_and_the_quit_hint_does_the_same` |
| `diff.next` | `n` selects the next hunk | `diff::n_and_p_select_the_next_and_the_previous_hunk`, `diff::clicking_the_next_and_previous_hunk_hints_moves_between_hunks`, `diff::moving_to_a_hunk_scrolls_the_panes_to_it`, `diff::the_stacked_column_follows_the_selected_hunk` |
| `diff.previous` | `p` selects the previous hunk | `diff::n_and_p_select_the_next_and_the_previous_hunk`, `diff::clicking_the_next_and_previous_hunk_hints_moves_between_hunks` |
| `diff.next-conflict` | `N` jumps to the next unresolved conflict and says when there is none | `diff::capital_n_jumps_to_the_next_unresolved_conflict_and_says_when_there_is_none` |
| `diff.previous-conflict` | `P` jumps to the previous unresolved conflict | `diff::capital_p_jumps_to_the_previous_unresolved_conflict_and_says_when_there_is_none` |
| `diff.layout` | `v` cycles the layouts | `diff::v_cycles_five_columns_two_panes_and_the_stacked_column`, `diff::clicking_the_layout_hint_cycles_the_layout` |
| `diff.focus-next` | Tab moves the focus to the next pane | `diff::tab_and_shift_tab_move_the_pane_focus` |
| `diff.focus-previous` | Shift+Tab moves it back | `diff::tab_and_shift_tab_move_the_pane_focus` |
| `diff.widen` | `]` widens the focused pane | `diff::bracket_keys_widen_and_narrow_the_focused_pane`, `diff::dragging_a_divider_resizes_the_panes` |
| `diff.narrow` | `[` narrows it | `diff::bracket_keys_widen_and_narrow_the_focused_pane`, `diff::dragging_a_divider_resizes_the_panes` |
| `diff.down` | Down scrolls the panes together | `diff::down_and_up_scroll_every_pane_together`, `diff::the_mouse_wheel_scrolls_the_panes` |
| `diff.up` | Up scrolls back | `diff::down_and_up_scroll_every_pane_together`, `diff::the_mouse_wheel_scrolls_the_panes` |
| `diff.mine` | `o` takes my side for the hunk | `diff::o_keeps_mine_for_the_hunk_and_the_plan_writes_nothing`, `diff::the_choice_keys_have_click_targets_in_the_header` |
| `diff.theirs` | `t` takes upstream's side for the hunk | `diff::t_takes_theirs_for_the_hunk_and_write_puts_it_on_disk_with_a_backup_of_the_original`, `diff::the_choice_keys_have_click_targets_in_the_header` |
| `diff.both` | `b` takes both sides | `diff::b_keeps_both_sides_in_the_result_and_write_puts_both_on_disk`, `diff::clicking_both_in_the_header_keeps_both_sides` |
| `diff.edit` | `e` edits the hunk | `diff::e_hands_the_hunk_to_the_editor_and_write_puts_what_it_saved_on_disk`, `diff::clicking_edit_in_the_header_runs_the_editor` |
| `diff.write` | `w` writes the result | `diff::t_takes_theirs_for_the_hunk_and_write_puts_it_on_disk_with_a_backup_of_the_original`, `diff::the_write_hint_opens_the_plan_for_the_result` |

## Plan dialog

| Id | Interaction | Tests |
|---|---|---|
| `plan.confirm` | `y` or Enter on Confirm writes | `plan::y_confirms_and_writes_the_file_a_backup_and_one_transaction`, `plan::enter_on_the_confirm_button_writes_after_tab_moves_the_focus_to_it`, `plan::clicking_the_confirm_button_writes` |
| `plan.cancel` | `n`, `q`, Esc or Enter on Cancel writes nothing | `plan::n_cancels_and_changes_nothing`, `plan::q_cancels_the_plan_instead_of_quitting_and_changes_nothing`, `plan::escape_cancels_and_changes_nothing`, `plan::enter_on_the_default_cancel_button_changes_nothing`, `plan::clicking_the_cancel_button_changes_nothing` |
| `plan.focus` | Tab, Left, Right, `h` and `l` move between the buttons | `plan::tab_arrows_h_and_l_move_between_the_buttons` |
| `plan.select` | `j`, `k`, arrows, Home and End select a file | `plan::j_k_arrows_home_and_end_select_a_file_in_the_plan`, `plan::clicking_a_file_in_the_plan_selects_it` |
| `plan.expand` | `d` expands or collapses the selected file's diff | `plan::d_expands_the_selected_files_diff_and_a_second_d_collapses_it` |

## Window and theme

| Id | Interaction | Tests |
|---|---|---|
| `window.resize` | resizing keeps the screen, the selection and an open dialog and repaints at the new size | planned |
| `window.resize-diff` | resizing across a width boundary switches the diff layout and keeps the selected hunk | `diff::resizing_across_a_width_boundary_follows_the_width_and_keeps_the_hunk` |
| `window.layouts` | the diff shows five columns (hunks, base, ours, theirs, result) from 200 columns, ours and theirs side by side with the result underneath from 100 columns, and one stacked column below that | `diff::from_200_columns_the_diff_opens_in_five_columns`, `diff::from_100_columns_the_diff_opens_with_ours_and_theirs_side_by_side_and_the_result_below`, `diff::below_100_columns_the_diff_stacks_in_one_column`, `diff::the_layout_edges_are_at_exactly_200_and_100_columns` |
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
