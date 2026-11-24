|ctx: &mut Ctx,
                 ui: &mut Ui,
                 group: Option<&str>,
                 group_entry_id: Option<&str>,
                 parent_enabled: bool,
                 row_index: usize,
                 mc: &mut ModConfig,
                 overrides: Option<&mut MemberOverrides>| {
                    let mut enabled = overrides.as_ref().map_or(mc.enabled, |o| o.enabled(mc));
                    let group_control_tint = group
                        .and_then(|name| group_tint_snapshot.get(name))
                        .and_then(|color| color.as_deref())
                        .map(|color| {
                            mod_list_style::swatch_color(Some(color), ui.visuals().dark_mode)
                        });
                    if !enabled {
                        let vis = ui.visuals_mut();
                        vis.override_text_color = Some(vis.text_color());
                        vis.hyperlink_color = vis.text_color();
                    }

                    if ui
                        .add(toggle_switch(&mut enabled))
                        .on_hover_text_at_pointer("Enabled?")
                        .changed()
                    {
                        if let Some(overrides) = overrides {
                            overrides.set(mc, enabled);
                        } else {
                            mc.enabled = enabled;
                        }
                        ctx.needs_save = true;
                    }

                    /*
                    if ui
                        .add(egui::Checkbox::without_text(&mut mc.required))
                        .changed()
                    {
                        needs_save = true;
                    }
                    */

                    if issues.duplicate(group, row_index)
                        && icon_buttons::warning_button(ui, group_control_tint)
                            .on_hover_text("Remove this duplicate entry")
                            .clicked()
                    {
                        ctx.btn_remove = Some((group.map(str::to_owned), row_index));
                    }

                    let info = self.state.store.get_mod_info(&mc.spec);
                    let unavailable = self.state.store.is_modio_unavailable(&mc.spec);
                    let problematic = info
                        .as_ref()
                        .and_then(|info| info.modio_id)
                        .is_some_and(|id| self.problematic_mod_id == Some(id));

                    // Display one error icon even when both conditions apply. Keep the distinction
                    // in the tooltip and leave duplicate/dependency warnings unchanged.
                    if unavailable || problematic {
                        let message = match (unavailable, problematic) {
                            (true, true) => "Unavailable on mod.io (cached data and files preserved). This mod was also flagged as problematic.",
                            (true, false) => "Unavailable on mod.io. Cached metadata and downloaded mod files were preserved; other mod updates can continue.",
                            (false, true) => "This mod was flagged as problematic.",
                            (false, false) => unreachable!(),
                        };
                        icon_buttons::mod_error_badge(ui).on_hover_text(message);
                    }
                    if enabled && let Some(req) = &self.integrate_rid {
                        let (accent, track) = if ui.visuals().dark_mode {
                            (egui::Color32::from_rgb(49, 126, 89), egui::Color32::from_rgb(65, 68, 72))
                        } else {
                            (egui::Color32::from_rgb(45, 119, 82), egui::Color32::from_rgb(220, 223, 225))
                        };
                        ui.scope(|ui| {
                            ui.visuals_mut().extreme_bg_color = track;
                            let bar = |fraction| {
                                egui::ProgressBar::new(fraction)
                                    .desired_width(100.0)
                                    .fill(accent)
                                    .corner_radius(4_u8)
                            };
                            match req.state.get(&mc.spec) {
                                Some(SpecFetchProgress::Progress { progress, size }) => {
                                    let fraction = if *size == 0 {
                                        0.0
                                    } else {
                                        *progress as f32 / *size as f32
                                    };
                                    ui.add(bar(fraction).show_percentage());
                                }
                                Some(SpecFetchProgress::Complete) => {
                                    ui.add(bar(1.0));
                                }
                                None => {
                                    ui.add(egui::Spinner::new().color(accent));
                                }
                            }
                        });
                    }

                    if let Some(info) = &info {
                        let previous_spec = mc.spec.clone();
                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                            egui::ComboBox::from_id_salt(row_index)
                                .selected_text(
                                    self.state
                                        .store
                                        .get_version_name(&mc.spec)
                                        .unwrap_or_default(),
                                )
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut mc.spec.url,
                                        info.spec.url.to_string(),
                                        self.state
                                            .store
                                            .get_version_name(&info.spec)
                                            .unwrap_or_default(),
                                    );
                                    for version in info.versions.iter().rev() {
                                        ui.selectable_value(
                                            &mut mc.spec.url,
                                            version.url.to_string(),
                                            self.state
                                                .store
                                                .get_version_name(version)
                                                .unwrap_or_default(),
                                        );
                                    }
                                });
                        });

                        ctx.needs_save |= mc.spec != previous_spec;

                        ui.scope(|ui| {
                            mod_list_style::tint_group_member_controls(ui, group_control_tint);
                        ui.style_mut().spacing.interact_size.x = 30.;
                        let dark = ui.visuals().dark_mode;
                        match priority_preview_value(ui, mc.priority).cmp(&0) {
                            std::cmp::Ordering::Less => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_RED
                                } else {
                                    Color32::DARK_RED
                                });
                            }
                            std::cmp::Ordering::Greater => {
                                ui.visuals_mut().override_text_color = Some(if dark {
                                    Color32::LIGHT_GREEN
                                } else {
                                    Color32::DARK_GREEN
                                });
                            }
                            _ => {}
                        }
                        let previous_priority = mc.priority;
                        priority_drag_value(ui, &mut mc.priority)
                            .on_hover_text_at_pointer("Load Priority\nIn case of asset conflict, mods with higher priority take precedent.\nCan have duplicate values.",);
                        if mc.priority != previous_priority {
                            ctx.needs_save = true;
                        }
                    });

                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        if enabled && parent_enabled {
                            // ModHub requirements can be fulfilled by one enabled alternative.
                            let missing_deps = issues.missing_dependencies(group_entry_id, &mc.id);

                            if !missing_deps.is_empty() {
                                let mut msg = "Add missing dependencies:".to_string();
                                for dep in missing_deps {
                                    msg.push('\n');
                                    if let Some(rule) = dep.replacement_rule() {
                                        if rule.dependency_warning.is_empty() {
                                            msg.push_str(&format!("{} (recommended: {})", rule.original_name, rule.replacement_name));
                                        } else {
                                            msg.push_str(&rule.dependency_warning);
                                        }
                                    } else {
                                        msg.push_str(&dep.url);
                                    }
                                }
                                if icon_buttons::warning_button(ui, group_control_tint)
                                    .on_hover_text(msg)
                                    .clicked()
                                {
                                    ctx.add_deps = Some(missing_deps.iter()
                                        .map(|dep| if let Some(rule) = dep.replacement_rule() {
                                            ModSpecification::new(rule.replacement.clone())
                                        } else {
                                            dep.clone()
                                        })
                                        .collect());
                                }
                            }
                        }

                        match info.provider {
                            "modio" => {
                                let texture: &egui::TextureHandle =
                                    self.modio_texture_handle.get_or_insert_with(|| {
                                        let image =
                                            image::load_from_memory(MODIO_LOGO_PNG).unwrap();
                                        let size = [image.width() as _, image.height() as _];
                                        let image_buffer = image.to_rgba8();
                                        let pixels = image_buffer.as_flat_samples();
                                        let image = egui::ColorImage::from_rgba_unmultiplied(
                                            size,
                                            pixels.as_slice(),
                                        );

                                        ui.ctx().load_texture(
                                            "modio-logo",
                                            image,
                                            Default::default(),
                                        )
                                    });
                                let mut img = egui::Image::new(texture)
                                    .fit_to_exact_size([16.0, 16.0].into());
                                if !enabled {
                                    img = img.tint(Color32::LIGHT_RED);
                                }
                                ui.add(img);
                            }
                            "http" => {
                                ui.label("🌐");
                            }
                            "file" => {
                                icon_buttons::folder_badge(ui, group_control_tint);
                            }
                            _ => unimplemented!("unimplemented provider kind"),
                        }

                        let search = searchable_text(&info.name, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }

                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if let Some(description) = mc
                                .description
                                .as_deref()
                                .filter(|text| !text.trim().is_empty())
                            {
                                description_help_button(ui, description);
                            }
                            mod_tags::render_mod_tags(
                                ui,
                                info,
                                &self.search_string,
                                &mut ctx.scroll_to_match,
                            );
                        });
                    } else {
                        if icon_buttons::copy_url_button(ui, group_control_tint)
                            .on_hover_text_at_pointer("Copy URL")
                            .clicked()
                        {
                            ui.ctx().copy_text(mc.spec.url.to_string());
                        }

                        let search = searchable_text(&mc.spec.url, &self.search_string, {
                            TextFormat {
                                color: ui.visuals().hyperlink_color,
                                ..Default::default()
                            }
                        });

                        let res = ui.hyperlink_to(search.job, &mc.spec.url);
                        if search.is_match && ctx.scroll_to_match {
                            res.scroll_to_me(None);
                            ctx.scroll_to_match = false;
                        }
                        if let Some(description) = mc
                            .description
                            .as_deref()
                            .filter(|text| !text.trim().is_empty())
                        {
                            description_help_button(ui, description);
                        }
                    }
                }
