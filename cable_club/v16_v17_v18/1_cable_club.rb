class PokeBattle_Trainer
  attr_writer :online_trainer_type
  def online_trainer_type
    return @online_trainer_type || self.trainertype
  end
  
  attr_writer :online_win_text
  def online_win_text
    return @online_win_text || 0
  end
  attr_writer :online_lose_text
    def online_lose_text
    return @online_lose_text || 0
  end
end

module CableClub
  ACTIVITY_OPTIONS = {:battle => _INTL("battle"),
                      :trade => _INTL("trade"),
                      :record_mix => _INTL("mix records")}
  def self.pokemon_order(client_id)
    case client_id
    when 0; [0, 1, 2, 3, 4, 5]
    when 1; [1, 0, 3, 2, 5, 4]
    else; raise "Unknown client_id: #{client_id}"
    end
  end

  def self.pokemon_target_order(client_id)
    case client_id
    when 0..1; [1, 0, 3, 2, 5, 4]
    else; raise "Unknown client_id: #{client_id}"
    end
  end

  # Renamed constants, yay...
  if !defined?(ESSENTIALSVERSION) && !defined?(ESSENTIALS_VERSION)
    def self.do_battle(connection, client_id, seed, battle_rules, player_party, partner, partner_party)
      pbHealAll # Avoids having to transmit damaged state.
      partner_party.each {|pkmn| pkmn.heal}
      olditems=player_party.transform{|p| p.item }
      olditems2=partner_party.transform{|p| p.item }
      if !DISABLE_SKETCH_ONLINE
        oldmoves  = $player.party.transform { |p| p.moves.dup }
        oldmoves2 = partner_party.transform { |p| p.moves.dup }
      end
      oldlevels = nil
      oldlevels=battle_rules.adjustLevels(player_party,partner_party)
      scene = pbNewBattleScene
      battle = PokeBattle_CableClub.new(connection, client_id, scene, player_party, partner_party, partner, seed)
      battle.fullparty1 = battle.fullparty2 = true
      battle.endspeechwin = _INTL(ONLINE_WIN_SPEECHES_LIST[partner.online_win_text])
      battle.endspeech = _INTL(ONLINE_LOSE_SPEECHES_LIST[partner.online_lose_text])
      battle.items = []
      battle.internalbattle = false
      battle_rules.applyBattleRules(battle)
      trainerbgm = pbGetTrainerBattleBGM(partner)
      Events.onStartBattle.trigger(nil, nil)
      pbPrepareBattle(battle)
      # XXX: Configuring Online Battle Rules
      battle.weather = 0
      battle.weatherduration = 0
      battle.environment = PBEnvironment::None
      exc = nil
      pbBattleAnimation(trainerbgm, partner.trainertype, partner.name) {
        pbSceneStandby {
          # XXX: Hope we call rand in the same order in both clients...
          begin
            battle.pbStartBattle(true)
          rescue Connection::Disconnected, Errno::ECONNABORTED
            scene.pbEndBattle(0)
            exc = $!
          ensure
            battle_rules.unadjustLevels(player_party,partner_party,oldlevels)
            player_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems[i]
              pkmn.moves = oldmoves[i] if !DISABLE_SKETCH_ONLINE
            end
            partner_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems2[i]
              pkmn.moves = oldmoves2[i] if !DISABLE_SKETCH_ONLINE
            end
          end
        }
      }
      raise exc if exc
    end

    def self.do_trade(index, you, your_pkmn)
      my_pkmn = $Trainer.party[index]
      your_pkmn.obtainMode = 2 # traded
      $Trainer.seen[your_pkmn.species] = true
      $Trainer.owned[your_pkmn.species] = true
      pbSeenForm(your_pkmn)
      pbFadeOutInWithMusic(99999) {
        scene = PokemonTradeScene.new
        scene.pbStartScreen(my_pkmn, your_pkmn, $Trainer.name, you.name)
        scene.pbTrade
        scene.pbEndScreen
      }
      $Trainer.party[index] = your_pkmn
    end

    def self.choose_pokemon
      chosen = -1
      pbFadeOutIn(99999) {
        scene = PokemonScreen_Scene.new
        screen = PokemonScreen.new(scene, $Trainer.party)
        screen.pbStartScene(_INTL("Choose a Pokémon."), false)
        chosen = screen.pbChoosePokemon
        screen.pbEndScene
      }
      return chosen
    end
    
    def self.choose_team(ruleset)
      team_order = nil
      pbFadeOutIn(99999) {
        scene = PokemonScreen_Scene.new
        screen = PokemonScreen.new(scene, $Trainer.party)
        team_order = screen.pbPokemonMultipleEntryScreenOrder(ruleset)
      }
      return team_order
    end
    
    def self.check_pokemon(pkmn)
      pbFadeOutIn(99999) {
        scene = PokemonSummaryScene.new
        screen = PokemonSummary.new(scene)
        screen.pbStartScreen([pkmn],0)
      }
    end
  elsif defined?(ESSENTIALSVERSION) && ESSENTIALSVERSION =~ /^17/
    def self.do_battle(connection, client_id, seed, battle_rules, player_party, partner, partner_party)
      pbHealAll # Avoids having to transmit damaged state.
      partner_party.each {|pkmn| pkmn.heal}
      olditems=player_party.transform{|p| p.item }
      olditems2=partner_party.transform{|p| p.item }
      if !DISABLE_SKETCH_ONLINE
        oldmoves  = $player.party.transform { |p| p.moves.dup }
        oldmoves2 = partner_party.transform { |p| p.moves.dup }
      end
      oldlevels = nil
      oldlevels=battle_rules.adjustLevels(player_party,partner_party)
      scene = pbNewBattleScene
      battle = PokeBattle_CableClub.new(connection, client_id, scene, player_party, partner_party, partner, seed)
      battle.fullparty1 = battle.fullparty2 = true
      battle.endspeechwin = _INTL(ONLINE_WIN_SPEECHES_LIST[partner.online_win_text])
      battle.endspeech = _INTL(ONLINE_LOSE_SPEECHES_LIST[partner.online_lose_text])
      battle.items = []
      battle.internalbattle = false
      battle_rules.applyBattleRules(battle)
      trainerbgm = pbGetTrainerBattleBGM(partner)
      Events.onStartBattle.trigger(nil, nil)
      pbPrepareBattle(battle)
      # XXX: Configuring Online Battle Rules
      battle.weather = 0
      battle.weatherduration = 0
      battle.environment = PBEnvironment::None
      exc = nil
      pbBattleAnimation(trainerbgm, battle.doublebattle ? 3 : 1, [partner]) {
        pbSceneStandby {
          # XXX: Hope we call rand in the same order in both clients...
          begin
            battle.pbStartBattle(true)
          rescue Connection::Disconnected, Errno::ECONNABORTED
            scene.pbEndBattle(0)
            exc = $!
          ensure
            battle_rules.unadjustLevels(player_party,partner_party,oldlevels)
            player_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems[i]
              pkmn.moves = oldmoves[i] if !DISABLE_SKETCH_ONLINE
            end
            partner_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems2[i]
              pkmn.moves = oldmoves2[i] if !DISABLE_SKETCH_ONLINE
            end
          end
        }
      }
      raise exc if exc
    end

    def self.do_trade(index, you, your_pkmn)
      my_pkmn = $Trainer.party[index]
      your_pkmn.obtainMode = 2 # traded
      $Trainer.seen[your_pkmn.species] = true
      $Trainer.owned[your_pkmn.species] = true
      pbSeenForm(your_pkmn)
      pbFadeOutInWithMusic(99999) {
        scene = PokemonTrade_Scene.new
        scene.pbStartScreen(my_pkmn, your_pkmn, $Trainer.name, you.name)
        scene.pbTrade
        scene.pbEndScreen
      }
      $Trainer.party[index] = your_pkmn
    end

    def self.choose_pokemon
      chosen = -1
      pbFadeOutIn(99999) {
        scene = PokemonParty_Scene.new
        screen = PokemonPartyScreen.new(scene, $Trainer.party)
        screen.pbStartScene(_INTL("Choose a Pokémon."), false)
        chosen = screen.pbChoosePokemon
        screen.pbEndScene
      }
      return chosen
    end
    
    def self.choose_team(ruleset)
      team_order = nil
      pbFadeOutIn(99999) {
        scene = PokemonParty_Scene.new
        screen = PokemonPartyScreen.new(scene, $Trainer.party)
        team_order = screen.pbPokemonMultipleEntryScreenOrder(ruleset)
      }
      return team_order
    end
    
    def self.check_pokemon(pkmn)
      pbFadeOutIn(99999) {
        scene = PokemonSummary_Scene.new
        screen = PokemonSummaryScreen.new(scene)
        screen.pbStartScreen([pkmn],0)
      }
    end
  elsif defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
    def self.do_battle(connection, client_id, seed, battle_rules, player_party, partner, partner_party)
      pbHealAll # Avoids having to transmit damaged state.
      partner_party.each {|pkmn| pkmn.heal} # back to back battles desync without it.
      olditems=player_party.transform{|p| p.item }
      olditems2=partner_party.transform{|p| p.item }
      if !DISABLE_SKETCH_ONLINE
        oldmoves  = $player.party.transform { |p| p.moves.dup }
        oldmoves2 = partner_party.transform { |p| p.moves.dup }
      end
      oldlevels = nil
      oldlevels=battle_rules.adjustLevels(player_party,partner_party)
      scene = pbNewBattleScene
      battle = PokeBattle_CableClub.new(connection, client_id, scene, player_party, partner_party, partner, seed)
      battle.endSpeechesWin = [_INTL(ONLINE_WIN_SPEECHES_LIST[partner.online_win_text])]
      battle.endSpeeches = [_INTL(ONLINE_LOSE_SPEECHES_LIST[partner.online_lose_text])]
      battle.items = []
      battle.internalBattle = false
      battle_rules.applyBattleRules(battle)
      trainerbgm = pbGetTrainerBattleBGM(partner)
      Events.onStartBattle.trigger(nil, nil)
      # XXX: Configuring Online Battle Rules
      setBattleRule("environment", PBEnvironment::None)
      setBattleRule("weather", PBWeather::None)
      setBattleRule("terrain", PBBattleTerrains::None)
      setBattleRule("backdrop", "indoor1")
      pbPrepareBattle(battle)
      $PokemonTemp.clearBattleRules
      exc = nil
      pbBattleAnimation(trainerbgm, (battle.singleBattle?) ? 1 : 3, [partner]) {
        pbSceneStandby {
          # XXX: Hope we call rand in the same order in both clients...
          begin
            battle.pbStartBattle
          rescue Connection::Disconnected, Errno::ECONNABORTED
            scene.pbEndBattle(0)
            exc = $!
          ensure
            battle_rules.unadjustLevels(player_party,partner_party,oldlevels)
            player_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems[i]
              pkmn.moves = oldmoves[i] if !DISABLE_SKETCH_ONLINE
            end
            partner_party.each_with_index do |pkmn, i|
              pkmn.heal
              pkmn.makeUnmega
              pkmn.makeUnprimal
              pkmn.item = olditems2[i]
              pkmn.moves = oldmoves2[i] if !DISABLE_SKETCH_ONLINE
            end
          end
        }
      }
      raise exc if exc
    end

    def self.do_trade(index, you, your_pkmn)
      my_pkmn = $Trainer.party[index]
      your_pkmn.obtainMode = 2 # traded
      $Trainer.seen[your_pkmn.species] = true
      $Trainer.owned[your_pkmn.species] = true
      pbSeenForm(your_pkmn)
      pbFadeOutInWithMusic(99999) {
        scene = PokemonTrade_Scene.new
        scene.pbStartScreen(my_pkmn, your_pkmn, $Trainer.name, you.name)
        scene.pbTrade
        scene.pbEndScreen
      }
      $Trainer.party[index] = your_pkmn
    end

    def self.choose_pokemon
      chosen = -1
      pbFadeOutIn(99999) {
        scene = PokemonParty_Scene.new
        screen = PokemonPartyScreen.new(scene, $Trainer.party)
        screen.pbStartScene(_INTL("Choose a Pokémon."), false)
        chosen = screen.pbChoosePokemon
        screen.pbEndScene
      }
      return chosen
    end
    
    def self.choose_team(ruleset)
      team_order = nil
      pbFadeOutIn(99999) {
        scene = PokemonParty_Scene.new
        screen = PokemonPartyScreen.new(scene, $Trainer.party)
        team_order = screen.pbPokemonMultipleEntryScreenOrder(ruleset)
      }
      return team_order
    end
    
    def self.check_pokemon(pkmn)
      pbFadeOutIn(99999) {
        scene = PokemonSummary_Scene.new
        screen = PokemonSummaryScreen.new(scene)
        screen.pbStartScreen([pkmn],0,true)
      }
    end
  end

  def self.write_party(writer)
    writer.int($Trainer.party.length)
    $Trainer.party.each do |pkmn|
      write_pkmn(writer, pkmn)
    end
  end

  def self.write_pkmn(writer, pkmn)
    is_v18 = defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
    writer.int(pkmn.species)
    writer.int(pkmn.level)
    writer.int(pkmn.personalID)
    writer.int(pkmn.trainerID)
    writer.str(pkmn.ot)
    writer.int(pkmn.otgender)
    writer.int(pkmn.language)
    writer.int(pkmn.exp)
    writer.int(pkmn.form)
    writer.int(pkmn.item)
    writer.int(pkmn.moves.length)
    pkmn.moves.each do |move|
      writer.int(move.id)
      writer.int(move.ppup)
    end
    writer.int(pkmn.firstmoves.length)
    pkmn.firstmoves.each do |move|
      writer.int(move)
    end
    # in hindsight, don't really need to send the calculated values
    writer.nil_or(:int, pkmn.genderflag)
    writer.nil_or(:bool, pkmn.shinyflag)
    writer.nil_or(:int, pkmn.abilityflag)
    writer.nil_or(:int, pkmn.natureflag)
    writer.nil_or(:int, pkmn.natureOverride) if is_v18
    for i in 0...6
      writer.int(pkmn.iv[i])
      writer.nil_or(:bool, pkmn.ivMaxed[i]) if is_v18
      writer.int(pkmn.ev[i])
    end
    writer.int(pkmn.happiness)
    writer.str(pkmn.name)
    writer.int(pkmn.ballused)
    writer.int(pkmn.eggsteps)
    writer.nil_or(:int,pkmn.pokerus)
    writer.int(pkmn.obtainMap)
    writer.nil_or(:str,pkmn.obtainText)
    writer.int(pkmn.obtainLevel)
    writer.int(pkmn.obtainMode)
    writer.int(pkmn.hatchedMap)
    writer.int(pkmn.cool)
    writer.int(pkmn.beauty)
    writer.int(pkmn.cute)
    writer.int(pkmn.smart)
    writer.int(pkmn.tough)
    writer.int(pkmn.sheen)
    writer.int(pkmn.ribbonCount)
    pkmn.ribbons.each do |ribbon|
      writer.int(ribbon)
    end
    writer.bool(!!pkmn.mail)
    if pkmn.mail
      writer.int(pkmn.mail.item)
      writer.str(pkmn.mail.message)
      writer.str(pkmn.mail.sender)
      if pkmn.mail.poke1
        #[species,gender,shininess,form,shadowness,is egg]
        writer.int(pkmn.mail.poke1[0])
        writer.int(pkmn.mail.poke1[1])
        writer.bool(pkmn.mail.poke1[2])
        writer.int(pkmn.mail.poke1[3])
        writer.bool(pkmn.mail.poke1[4])
        writer.bool(pkmn.mail.poke1[5])
      else
        writer.nil_or(:int,nil)
      end
      if pkmn.mail.poke2
        #[species,gender,shininess,form,shadowness,is egg]
        writer.int(pkmn.mail.poke2[0])
        writer.int(pkmn.mail.poke2[1])
        writer.bool(pkmn.mail.poke2[2])
        writer.int(pkmn.mail.poke2[3])
        writer.bool(pkmn.mail.poke2[4])
        writer.bool(pkmn.mail.poke2[5])
      else
        writer.nil_or(:int,nil)
      end
      if pkmn.mail.poke3
        #[species,gender,shininess,form,shadowness,is egg]
        writer.int(pkmn.mail.poke3[0])
        writer.int(pkmn.mail.poke3[1])
        writer.bool(pkmn.mail.poke3[2])
        writer.int(pkmn.mail.poke3[3])
        writer.bool(pkmn.mail.poke3[4])
        writer.bool(pkmn.mail.poke3[5])
      else
        writer.nil_or(:int,nil)
      end
    end
    writer.bool(!!pkmn.fused)
    if pkmn.fused
      write_pkmn(writer, pkmn.fused)
    end
    if defined?(EliteBattle) # EBDX compat
      # this looks so dumb I know, but the variable can be nil, false, or an int.
      writer.bool(pkmn.shiny?)
      writer.str(pkmn.superHue.to_s)
      writer.nil_or(:bool,pkmn.superVariant)
    end
  end

  def self.parse_party(record)
    party = []
    record.int.times do
      party << parse_pkmn(record)
    end
    return party
  end

  def self.parse_pkmn(record)
    is_v18 = defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
    species = record.int
    level = record.int
    pkmn = PokeBattle_Pokemon.new(species, level, $Trainer)
    pkmn.personalID = record.int
    pkmn.trainerID = record.int
    pkmn.ot = record.str
    pkmn.otgender = record.int
    pkmn.language = record.int
    pkmn.exp = record.int
    form = record.int
    if is_v18
      pkmn.formSimple = form
    else
      pkmn.formNoCall = form
    end
    pkmn.setItem(record.int)
    pkmn.resetMoves
    for i in 0...record.int
      pkmn.moves[i] = PBMove.new(record.int)
      pkmn.moves[i].ppup = record.int
    end
    pkmn.firstmoves = []
    for i in 0...record.int
      pkmn.firstmoves.push(record.int)
    end
    pkmn.genderflag = record.nil_or(:int)
    pkmn.shinyflag = record.nil_or(:bool)
    pkmn.abilityflag = record.nil_or(:int)
    pkmn.natureflag = record.nil_or(:int)
    pkmn.natureOverride = record.nil_or(:int) if is_v18
    for i in 0...6
      pkmn.iv[i] = record.int
      pkmn.ivMaxed[i] = record.nil_or(:bool) if is_v18
      pkmn.ev[i] = record.int
    end
    pkmn.happiness = record.int
    pkmn.name = record.str
    pkmn.ballused = record.int
    pkmn.eggsteps = record.int
    pkmn.pokerus = record.nil_or(:int)
    pkmn.obtainMap = record.int
    pkmn.obtainText = record.nil_or(:str)
    pkmn.obtainLevel = record.int
    pkmn.obtainMode = record.int
    pkmn.hatchedMap = record.int
    pkmn.cool = record.int
    pkmn.beauty = record.int
    pkmn.cute = record.int
    pkmn.smart = record.int
    pkmn.tough = record.int
    pkmn.sheen = record.int
    pkmn.clearAllRibbons
    for i in 0...record.int
      pkmn.giveRibbon(record.int)
    end
    if record.bool() # mail
      m_item = record.int()
      m_msg = record.str()
      m_sender = record.str()
      m_poke1 = []
      if m_species1 = record.nil_or(:int)
        #[species,gender,shininess,form,shadowness,is egg]
        m_poke1[0] = m_species1
        m_poke1[1] = record.int()
        m_poke1[2] = record.bool()
        m_poke1[3] = record.int()
        m_poke1[4] = record.bool()
        m_poke1[5] = record.bool()
      else
        m_poke1 = nil
      end
      m_poke2 = []
      if m_species2 = record.nil_or(:int)
        #[species,gender,shininess,form,shadowness,is egg]
        m_poke2[0] = m_species2
        m_poke2[1] = record.int()
        m_poke2[2] = record.bool()
        m_poke2[3] = record.int()
        m_poke2[4] = record.bool()
        m_poke2[5] = record.bool()
      else
        m_poke2 = nil
      end
      m_poke3 = []
      if m_species3 = record.nil_or(:int)
        #[species,gender,shininess,form,shadowness,is egg]
        m_poke3[0] = m_species3
        m_poke3[1] = record.int()
        m_poke3[2] = record.bool()
        m_poke3[3] = record.int()
        m_poke3[4] = record.bool()
        m_poke3[5] = record.bool()
      else
        m_poke3 = nil
      end
      pkmn.mail = PokemonMail.new(m_item,m_msg,m_sender,m_poke1,m_poke2,m_poke3)
    end
    if record.bool()# fused
      pkmn.fused = parse_pkmn(record)
    end
    if defined?(EliteBattle) # EBDX compat
      # this looks so dumb I know, but the variable can be nil, false, or an int.
      record.bool # shiny call.
      superhue = record.str
      if superhue == ""
        pkmn.superHue = nil
      elsif superhue=="false"
        pkmn.superHue = false
      else
        pkmn.superHue = superhue.to_i
      end
      pkmn.superVariant = record.nil_or(:bool)
    end
    pkmn.calcStats
    return pkmn
  end
  
  def self.parse_battle_rules(record)
    rules = []
    record.int.times do
      rules << parse_battle_rule(record)
    end
    return rules
  end
  
  def self.parse_battle_rule(record)
    name = record.str
    desc = record.str
    rule = PokemonOnlineRules.new
    rule.setTeamPreview(record.int)
    rule.setNumberRange(record.int,record.int)
    # level adjustment
    level_adjustment = record.nil_or(:str)
    if level_adjustment
      level_adjustment_data = level_adjustment.split(";")
      level_adjustmentClass = level_adjustment_data.shift
      level_adjustment_args = process_args_type_hint(*level_adjustment_data)
      if Object.const_defined?(level_adjustmentClass)
        rule.setLevelAdjustment(Kernel.const_get(level_adjustmentClass),*level_adjustment_args)
      end
    end
    # battle rules
    record.int.times do
      clause_data = record.str.split(";")
      clauseClass = clause_data.shift
      clause_args = process_args_type_hint(*clause_data)
      if Object.const_defined?(clauseClass)
        rule.addBattleRule(Kernel.const_get(clauseClass),*clause_args)
      end
    end
    # pokemon rules
    record.int.times do
      clause_data = record.str.split(";")
      clauseClass = clause_data.shift
      clause_args = process_args_type_hint(*clause_data)
      if Object.const_defined?(clauseClass)
        rule.addPokemonRule(Kernel.const_get(clauseClass),*clause_args)
      end
    end
    # subset rules
    record.int.times do
      clause_data = record.str.split(";")
      clauseClass = clause_data.shift
      clause_args = process_args_type_hint(*clause_data)
      if Object.const_defined?(clauseClass)
        rule.addSubsetRule(Kernel.const_get(clauseClass),*clause_args)
      end
    end
    # team rules
    record.int.times do
      clause_data = record.str.split(";")
      clauseClass = clause_data.shift
      clause_args = process_args_type_hint(*clause_data)
      if Object.const_defined?(clauseClass)
        rule.addTeamRule(Kernel.const_get(clauseClass),*clause_args)
      end
    end
    return [name,desc,rule]
  end
  
  def self.write_battle_rule(writer,battle_rule)  
    name,desc,rule = battle_rule
    writer.str(name)
    writer.str(desc)
    writer.int(rule.team_preview)
    writer.int(rule.ruleset.minLength)
    writer.int(rule.ruleset.maxLength)
    if rule.rules_hash[:level_adjust]
      writer.str(rule.rules_hash[:level_adjust].join(";"))
    else
      writer.nil_or(:str,nil)
    end
    writer.int(rule.rules_hash[:battle].length)
    rule.rules_hash[:battle].each do |br|
      writer.str(br.join(";"))
    end
    writer.int(rule.rules_hash[:pokemon].length)
    rule.rules_hash[:pokemon].each do |pr|
      writer.str(pr.join(";"))
    end
    writer.int(rule.rules_hash[:subset].length)
    rule.rules_hash[:subset].each do |sr|
      writer.str(sr.join(";"))
    end
    writer.int(rule.rules_hash[:team].length)
    rule.rules_hash[:team].each do |tr|
      writer.str(tr.join(";"))
    end
  end
  
  def self.get_server_info
    ret = [HOST,PORT]
    if safeExists?("serverinfo.ini")
      File.foreach("serverinfo.ini") do |line|
        case line
        when /^\s*[Hh][Oo][Ss][Tt]\s*=\s*(.+)$/
          ret[0]=$1 if $1 && $1 != ""
        when /^\s*[Pp][Oo][Rr][Tt]\s*=\s*(\d{1,5})$/
          if $1 && $1 != ""
            port = $1.to_i
            ret[1]= port if port>0 && port<=65535
          end
        end
      end
    end
    return ret
  end
  
  # only handles int, bool, sym, and str
  def self.apply_args_type_hint(*args)
    ret = []
    args.each do |arg|
      case arg
      when Integer; ret.push([:int,arg])
      when TrueClass,FalseClass; ret.push([:bool,arg])
      when String; ret.push([:str,arg])
      when Symbol; ret.push([:sym,arg])
      end
    end
    return ret
  end
  
  # takes a long chain of args, every second element is the original argument
  def self.process_args_type_hint(*args)
    ret = []
    r = nil
    args.each do |arg|
      if r
        case r
        when :int; ret.push(arg.to_i)
        when :bool
          if arg == "true"
            ret.push(true)
          elsif arg == "false"
            ret.push(false)
          else
            raise "expected bool, got #{arg}"
          end
        when :str; ret.push(arg)
        when :sym; ret.push(arg.to_sym)
        end
        r = nil
      else
        r = arg.to_sym
      end
    end
    return ret
  end
  
end

module PokemonPartyCableClubAdditions
  def pbPokemonMultipleEntryScreenOrder(ruleset)
    annot = []
    statuses = []
    ordinals = [
       _INTL("INELIGIBLE"),
       _INTL("NOT ENTERED"),
       _INTL("BANNED"),
       _INTL("FIRST"),
       _INTL("SECOND"),
       _INTL("THIRD"),
       _INTL("FOURTH"),
       _INTL("FIFTH"),
       _INTL("SIXTH")
    ]
    return nil if !ruleset.hasValidTeam?(@party)
    ret = nil
    addedEntry = false
    for i in 0...@party.length
      statuses[i] = (ruleset.isPokemonValid?(@party[i])) ? 1 : 2
    end
    for i in 0...@party.length
      annot[i] = ordinals[statuses[i]]
    end
    @scene.pbStartScene(@party,_INTL("Choose Vinemon and confirm."),annot,true)
    loop do
      realorder = []
      for i in 0...@party.length
        for j in 0...@party.length
          if statuses[j]==i+3
            realorder.push(j)
            break
          end
        end
      end
      for i in 0...realorder.length
        statuses[realorder[i]] = i+3
      end
      for i in 0...@party.length
        annot[i] = ordinals[statuses[i]]
      end
      @scene.pbAnnotate(annot)
      if realorder.length==ruleset.number && addedEntry
        @scene.pbSelect(6)
      end
      @scene.pbSetHelpText(_INTL("Choose Pokémon and confirm."))
      pkmnid = @scene.pbChoosePokemon
      addedEntry = false
      if pkmnid==6 # Confirm was chosen
        ret = []
        test_ret = []
        for i in realorder
          ret.push(i)
          test_ret.push(@party[i])
        end
        error = []
        break if ruleset.isValid?(test_ret,error)
        pbDisplay(error[0])
        ret = nil
        test_ret = nil
      end
      break if pkmnid<0 # Canceled
      cmdEntry   = -1
      cmdNoEntry = -1
      cmdSummary = -1
      commands = []
      if (statuses[pkmnid] || 0) == 1
        commands[cmdEntry = commands.length]   = _INTL("Entry")
      elsif (statuses[pkmnid] || 0) > 2
        commands[cmdNoEntry = commands.length] = _INTL("No Entry")
      end
      pkmn = @party[pkmnid]
      commands[cmdSummary = commands.length]   = _INTL("Summary")
      commands[commands.length]                = _INTL("Cancel")
      command = @scene.pbShowCommands(_INTL("Do what with {1}?",pkmn.name),commands) if pkmn
      if cmdEntry>=0 && command==cmdEntry
        if realorder.length>=ruleset.number && ruleset.number>0
          pbDisplay(_INTL("No more than {1} Pokémon may enter.",ruleset.number))
        else
          statuses[pkmnid] = realorder.length+3
          addedEntry = true
          pbRefreshSingle(pkmnid)
        end
      elsif cmdNoEntry>=0 && command==cmdNoEntry
        statuses[pkmnid] = 1
        pbRefreshSingle(pkmnid)
      elsif cmdSummary>=0 && command==cmdSummary
        if !defined?(ESSENTIALSVERSION) && !defined?(ESSENTIALS_VERSION)
          @scene.pbSummary(pkmnid)
        else
          @scene.pbSummary(pkmnid,true)
        end
      end
    end
    @scene.pbEndScene
    return ret
  end
end

if !defined?(ESSENTIALSVERSION) && !defined?(ESSENTIALS_VERSION)
  class PokemonScreen
    include PokemonPartyCableClubAdditions
  end
else
  class PokemonPartyScreen
    include PokemonPartyCableClubAdditions
  end
end

if defined?(ESSENTIALSVERSION) && ESSENTIALSVERSION =~ /^17/
  class PokemonSummary_Scene
    alias _cable_club_pbStartScene pbStartScene
    def pbStartScene(party,partyindex,inbattle)
      @inbattle=inbattle
      _cable_club_pbStartScene(party,partyindex)
    end
    alias _cable_club_pbOptions pbOptions
    def pbOptions
      return false if @inbattle
      return _cable_club_pbOptions
    end
  end
  class PokemonSummaryScreen
    alias _cable_club_pbStartScreen pbStartScreen
    def pbStartScreen(party,partyindex,inbattle)
      @scene.pbStartScene(party,partyindex,inbattle)
      ret = @scene.pbScene
      @scene.pbEndScene
      return ret
    end
  end
  class PokemonParty_Scene
    def pbSummary(pkmnid,inbattle=false)
      oldsprites = pbFadeOutAndHide(@sprites)
      scene = PokemonSummary_Scene.new
      screen = PokemonSummaryScreen.new(scene)
      screen.pbStartScreen(@party,pkmnid,(inbattle || $game_temp.in_battle))
      pbFadeInAndShow(@sprites,oldsprites)
    end
  end
end

class PokeBattle_Battle
  attr_reader :client_id
end

class PokeBattle_CableClub < PokeBattle_Battle
  attr_reader :connection
  attr_reader :battleRNG
  def initialize(connection, client_id, scene, player_party, opponent_party, opponent, seed)
    @connection = connection
    @client_id = client_id
    online_back_check = pbPlayerSpriteBackFile($Trainer.online_trainer_type)
    if online_back_check && pbResolveBitmap(online_back_check)
      player = PokeBattle_Trainer.new($Trainer.name, $Trainer.online_trainer_type)
    else
      player = PokeBattle_Trainer.new($Trainer.name, $Trainer.trainertype)
    end
    super(scene, player_party, opponent_party, player, opponent)
    @battleAI  = PokeBattle_CableClub_AI.new(self) if defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
    @battleRNG = MersenneTwisterRandom.new(seed) # anti desync measures for EBS.
  end
  
  def pbRandom(x); return @battleRNG.rand(x); end
  
  # Added optional args to not make v18 break.
  def pbSwitchInBetween(index, lax=false, cancancel=false)
    if pbOwnedByPlayer?(index)
      choice = super(index, lax, cancancel)
      # bug fix for the unknown type :switch. cause: going into the pokemon menu then backing out and attacking, which sends the switch symbol regardless.
      if !cancancel # forced switches do not allow canceling, and both sides would expect a response.
        @connection.send do |writer|
          writer.sym(:switch)
          writer.int(choice)
        end
      end
      return choice
    else
      frame = 0
      # So much renamed stuff...
      if defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
        cbox = PokeBattle_Scene::MESSAGE_BOX
        hbox = "messageWindow"
        opponent = @opponent[0]
      else
        cbox = PokeBattle_Scene::MESSAGEBOX
        hbox = "messagewindow"
        opponent = @opponent
      end
      @scene.databoxVisible(false) rescue nil # EBS
      @scene.pbShowWindow(cbox)
      cw = @scene.sprites[hbox]
      cw.letterbyletter = false
      begin
        loop do
          frame += 1
          cw.text = _INTL("Waiting" + "." * (1 + ((frame / 8) % 3)))
          @scene.pbFrameUpdate(cw)
          Graphics.update
          Input.update
          raise Connection::Disconnected.new("disconnected") if Input.trigger?(Input::B) && Kernel.pbConfirmMessageSerious("Would you like to disconnect?")
          @connection.update do |record|
            case (type = record.sym)
            when :forfeit
              pbSEPlay("Battle flee")
              pbDisplay(_INTL("{1} forfeited the match!", opponent.fullname))
              @decision = 1
              pbAbort

            when :switch
              return record.int

            else
              raise "Unknown message: #{type}"
            end
          end
        end
      ensure
        cw.letterbyletter = true
        @scene.clearMessageWindow rescue nil # EBS
        @scene.databoxVisible(true) rescue nil # EBS
      end
    end
  end

  def pbRun(idxPokemon, duringBattle=false)
    ret = super(idxPokemon, duringBattle)
    if ret == 1
      @connection.send do |writer|
        writer.sym(:forfeit)
      end
      @connection.discard(1)
    end
    return ret
  end

  # Rearrange the battlers into a consistent order, do the function, then restore the order.
  if defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
    def pbCalculatePriority(*args)
      battlers = @battlers.dup
      begin
        order = CableClub::pokemon_order(@client_id)
        order.each_with_index do |o,i|
          @battlers[i] = battlers[o]
        end
        return super(*args)
      ensure
        @battlers = battlers
      end
    end
    
    def pbCanShowCommands?(idxBattler)
      last_index = pbGetOpposingIndicesInOrder(0).reverse.last
      return true if last_index==idxBattler
      return super(idxBattler)
    end
    
    # avoid unnecessary checks and check in same order
    def pbEORSwitch(favorDraws=false)
      return if @decision>0 && !favorDraws
      return if @decision==5 && favorDraws
      pbJudge
      return if @decision>0
      # Check through each fainted battler to see if that spot can be filled.
      switched = []
      loop do
        switched.clear
        # check in same order
        battlers = []
        order = CableClub::pokemon_order(@client_id)
        order.each_with_index do |o,i|
          battlers[i] = @battlers[o]
        end
        battlers.each do |b|
          next if !b || !b.fainted?
          idxBattler = b.index
          next if !pbCanChooseNonActive?(idxBattler)
          if !pbOwnedByPlayer?(idxBattler)   # Opponent/ally is switching in
            next if wildBattle? && opposes?(idxBattler)   # Wild Pokémon can't switch
            idxPartyNew = pbSwitchInBetween(idxBattler)
            opponent = pbGetOwnerFromBattlerIndex(idxBattler)
            pbRecallAndReplace(idxBattler,idxPartyNew)
            switched.push(idxBattler)
          else
            idxPlayerPartyNew = pbGetReplacementPokemonIndex(idxBattler)   # Owner chooses
            pbRecallAndReplace(idxBattler,idxPlayerPartyNew)
            switched.push(idxBattler)
          end
        end
        break if switched.length==0
        pbPriority(true).each do |b|
          b.pbEffectsOnSwitchIn(true) if switched.include?(b.index)
        end
      end
    end
    
  else # v16/v17
    def pbSwitch(favorDraws=false)
      if !favorDraws
        return if @decision>0
      else
        return if @decision==5
      end
      pbJudge()
      return if @decision>0
      switched=[]
      for index in CableClub::pokemon_order(@client_id)
        next if index > 3 
        next if !@doublebattle && pbIsDoubleBattler?(index)
        next if @battlers[index] && !@battlers[index].isFainted?
        next if !pbCanChooseNonActive?(index)
        if !pbOwnedByPlayer?(index)
          if !pbIsOpposing?(index) || (@opponent && pbIsOpposing?(index))
            newenemy=pbSwitchInBetween(index,false,false)
            newenemyname=newenemy
            if newenemy>=0 && isConst?(pbParty(index)[newenemy].ability,PBAbilities,:ILLUSION)
              newenemyname=pbGetLastPokeInTeam(index)
            end
            opponent=pbGetOwner(index)
            pbRecallAndReplace(index,newenemy,newenemyname,false,false)
            switched.push(index)
          end
        else
          newpoke=pbSwitchInBetween(index,true,false)
          newpokename=newpoke
          if isConst?(@party1[newpoke].ability,PBAbilities,:ILLUSION)
            newpokename=pbGetLastPokeInTeam(index)
          end
          pbRecallAndReplace(index,newpoke,newpokename)
          switched.push(index)
        end
      end
      if switched.length>0
        priority=pbPriority
        for i in priority
          i.pbAbilitiesOnSwitchIn(true) if switched.include?(i.index)
        end
      end
    end
    
    def pbPriority(*args)
      battlers = @battlers.dup
      choices = @choices.dup
      begin
        order = CableClub::pokemon_order(@client_id)
        for i in 0..3
          @battlers[i] = battlers[order[i]]
          @choices[i] = choices[order[i]]
        end
        return super(*args)
      ensure
        @battlers = battlers
        @choices = choices
      end
    end
    
    # This is horrific. Basically, we need to force Essentials to look for
    # the RHS foe's move in all circumstances, otherwise we won't transmit
    # any moves for this turn and the battle will hang.
    def pbCanShowCommands?(index)
      super(index) || (index == 3 && Kernel.caller(1) =~ /pbCanShowCommands/)
    end
    
    def pbDefaultChooseEnemyCommand(index)
      our_indices = @doublebattle ? [0, 2] : [0]
      their_indices = @doublebattle ? [1, 3] : [1]
      # Sends our choices after they have all been locked in.
      if index == their_indices.last
        target_order = CableClub::pokemon_target_order(@client_id)
        @connection.send do |writer|
          writer.sym(:battle_data)
          # Send Seed
          cur_seed=@battleRNG.seed
          @battleRNG.seed(cur_seed)
          writer.sym(:seed)
          writer.int(cur_seed)
          # Send Extra Battle Mechanics
          writer.sym(:mechanic)
          # Mega Evolution
          mega=@megaEvolution[0][0]
          mega^=1 if mega>=0
          writer.int(mega)
          # Send Choices for Player's Mons
          for our_index in our_indices
            pkmn = @battlers[our_index]
            writer.sym(:choice)
            writer.int(@choices[our_index][0])
            writer.int(@choices[our_index][1])
            move = !!@choices[our_index][2]
            writer.nil_or(:bool, move)
            # -1 invokes the RNG, out of order (somehow?!) which causes desync.
            # But this is a single battle, so the only possible choice is the foe.
            if !@doublebattle && @choices[our_index][3] == -1
              @choices[our_index][3] = their_indices[0]
            end
            # Target from their POV.
            our_target = @choices[our_index][3]
            their_target = target_order[our_target] rescue our_target
            writer.int(their_target)
          end
        end
        frame = 0
        @scene.databoxVisible(false) rescue nil # EBS
        @scene.pbShowWindow(PokeBattle_Scene::MESSAGEBOX)
        cw = @scene.sprites["messagewindow"]
        cw.letterbyletter = false
        begin
          loop do
            frame += 1
            cw.text = _INTL("Waiting" + "." * (1 + ((frame / 8) % 3)))
            @scene.pbFrameUpdate(cw)
            Graphics.update
            Input.update
            raise Connection::Disconnected.new("disconnected") if Input.trigger?(Input::B) && Kernel.pbConfirmMessageSerious("Would you like to disconnect?")
            @connection.update do |record|
              case (type = record.sym)
              when :forfeit
                pbSEPlay("Battle flee")
                pbDisplay(_INTL("{1} forfeited the match!", @opponent.fullname))
                @decision = 1
                pbAbort
            
              when :battle_data
                loop do
                  case (t = record.sym)
                  when :seed
                    seed=record.int
                    @battleRNG.seed(seed) if @client_id==1
                  when :mechanic
                    @megaEvolution[1][0] = record.int
                  when :choice
                    their_index = their_indices.shift
                    partner_pkmn = @battlers[their_index]
                    @choices[their_index][0] = record.int
                    @choices[their_index][1] = record.int
                    move = record.nil_or(:bool)
                    if move
                      move = (@choices[their_index][1]<0) ? @struggle : partner_pkmn.moves[@choices[their_index][1]]
                    end
                    @choices[their_index][2] = move
                    @choices[their_index][3] = record.int
                    break if their_indices.empty?
                  else
                    raise "Unknown message: #{t}"
                  end
                end
                return

              else
                raise "Unknown message: #{type}"
              end
            end
          end
        ensure
          cw.letterbyletter = true
          @scene.clearMessageWindow rescue nil # EBS
          @scene.databoxVisible(true) rescue nil # EBS
        end
      end
    end

    def pbDefaultChooseNewEnemy(index, party)
      raise "Expected this to be unused."
    end
    
    def pbEndOfBattle(canlose=false)
      case @decision
      ##### WIN #####
      when 1
        PBDebug.log("")
        PBDebug.log("***Player won***")
        if @opponent
          @scene.pbTrainerBattleSuccess
          if @opponent.is_a?(Array)
            pbDisplayPaused(_INTL("{1} defeated {2} and {3}!",self.pbPlayer.name,@opponent[0].fullname,@opponent[1].fullname))
          else
            pbDisplayPaused(_INTL("{1} defeated\r\n{2}!",self.pbPlayer.name,@opponent.fullname))
          end
          @scene.pbShowOpponent(0)
          pbDisplayPaused(@endspeech.gsub(/\\[Pp][Nn]/,self.pbPlayer.name))
          if @opponent.is_a?(Array)
            @scene.pbHideOpponent
            @scene.pbShowOpponent(1)
            pbDisplayPaused(@endspeech2.gsub(/\\[Pp][Nn]/,self.pbPlayer.name))
          end
        end
      ##### LOSE, DRAW #####
      when 2, 5
        PBDebug.log("")
        PBDebug.log("***Player lost***") if @decision==2
        PBDebug.log("***Player drew with opponent***") if @decision==5
        if @decision==2
          @scene.pbShowOpponent(0)
          pbDisplayPaused(@endspeechwin.gsub(/\\[Pp][Nn]/,self.pbPlayer.name))
          if @opponent.is_a?(Array)
            @scene.pbHideOpponent
            @scene.pbShowOpponent(1)
            pbDisplayPaused(@endspeechwin2.gsub(/\\[Pp][Nn]/,self.pbPlayer.name))
          end
        end
      end
      @scene.pbEndBattle(@decision)
      for i in @battlers
        i.pbResetForm
      end
      for i in @party1
        i.setItem(i.itemInitial)
        i.itemInitial=i.itemRecycle=0
        i.belch=false
      end
      return @decision
    end
  end
end
if defined?(ESSENTIALS_VERSION) && ESSENTIALS_VERSION =~ /^18/
  class PokeBattle_CableClub_AI < PokeBattle_AI
    def pbDefaultChooseEnemyCommand(index)
      # Hurray for default methods. have to reverse it to show the expected order.
      our_indices = @battle.pbGetOpposingIndicesInOrder(1).reverse
      their_indices = @battle.pbGetOpposingIndicesInOrder(0).reverse
      # Sends our choices after they have all been locked in.
      if index == their_indices.last
        # TODO: patch this up to be index agnostic.
        # Would work fine if restricted to single/double battles
        target_order = CableClub::pokemon_target_order(@battle.client_id)
        @battle.connection.send do |writer|
          writer.sym(:battle_data)
          # Send Seed
          cur_seed=@battle.battleRNG.seed
          @battle.battleRNG.seed(cur_seed)
          writer.sym(:seed)
          writer.int(cur_seed)
          # Send Extra Battle Mechanics
          writer.sym(:mechanic)
          # Mega Evolution
          mega=@battle.megaEvolution[0][0]
          mega^=1 if mega>=0
          writer.int(mega)
          # Send Choices for Player's Mons
          for our_index in our_indices
            pkmn = @battle.battlers[our_index]
            writer.sym(:choice)
            # choice picked was changed to be a symbol now.
            writer.sym(@battle.choices[our_index][0])
            writer.int(@battle.choices[our_index][1])
            move = !!@battle.choices[our_index][2]
            writer.nil_or(:bool, move)
            # -1 invokes the RNG, out of order (somehow?!) which causes desync.
            # But this is a single battle, so the only possible choice is the foe.
            if @battle.singleBattle? && @battle.choices[our_index][3] == -1
              @battle.choices[our_index][3] = their_indices[0]
            end
            # Target from their POV.
            our_target = @battle.choices[our_index][3]
            their_target = target_order[our_target] rescue our_target
            writer.int(their_target)
          end
        end
        frame = 0
        @battle.scene.pbShowWindow(PokeBattle_Scene::MESSAGE_BOX)
        cw = @battle.scene.sprites["messageWindow"]
        cw.letterbyletter = false
        begin
          loop do
            frame += 1
            cw.text = _INTL("Waiting" + "." * (1 + ((frame / 8) % 3)))
            @battle.scene.pbFrameUpdate(cw)
            Graphics.update
            Input.update
            raise Connection::Disconnected.new("disconnected") if Input.trigger?(Input::B) && Kernel.pbConfirmMessageSerious("Would you like to disconnect?")
            @battle.connection.update do |record|
              case (type = record.sym)
              when :forfeit
                pbSEPlay("Battle flee")
                @battle.pbDisplay(_INTL("{1} forfeited the match!", @battle.opponent[0].fullname))
                @battle.decision = 1
                @battle.pbAbort
              
              when :battle_data
                loop do
                  case (t = record.sym)
                  when :seed
                    seed=record.int()
                    @battle.battleRNG.seed(seed) if @client_id==1
                  when :mechanic
                    @battle.megaEvolution[1][0] = record.int
                  when :choice
                    their_index = their_indices.shift
                    partner_pkmn = @battle.battlers[their_index]
                    @battle.choices[their_index][0] = record.sym
                    @battle.choices[their_index][1] = record.int
                    move = record.nil_or(:bool)
                    if move
                      move = (@battle.choices[their_index][1]<0) ? @battle.struggle : partner_pkmn.moves[@battle.choices[their_index][1]]
                    end
                    @battle.choices[their_index][2] = move
                    @battle.choices[their_index][3] = record.int
                    break if their_indices.empty?
                  else
                    raise "Unknown message: #{t}"
                  end
                end
                return

              else
                raise "Unknown message: #{type}"
              end
            end
          end
        ensure
          cw.letterbyletter = true
        end
      end
    end

    def pbDefaultChooseNewEnemy(index, party)
      raise "Expected this to be unused."
    end
  end
  
  #===============================================================================
  # This move permanently turns into the last move used by the target. (Sketch)
  #===============================================================================
  class PokeBattle_Move_05D
    alias _cc_pbMoveFailed? pbMoveFailed?
    def pbMoveFailed?(user, targets)
      if CableClub::DISABLE_SKETCH_ONLINE && !@battle.internalBattle
        @battle.pbDisplay(_INTL("But it failed!"))
        return true
      end
      return _cc_pbMoveFailed?(user, targets)
    end
  end
  
else
  class PokeBattle_Battler
    alias old_pbFindUser pbFindUser if !defined?(old_pbFindUser)

    # This ensures the targets are processed in the same order.
    def pbFindUser(choice, targets)
      ret = old_pbFindUser(choice, targets)
      if !@battle.client_id.nil?
        order = CableClub::pokemon_order(@battle.client_id)
        targets.sort! {|a, b| order[a.index] <=> order[b.index]}
      end
      return ret
    end
  end
  #===============================================================================
  # This move permanently turns into the last move used by the target. (Sketch)
  #===============================================================================
  class PokeBattle_Move_05D
    alias _cc_pbEffect pbEffect
    def pbEffect(attacker,opponent,hitnum=0,alltargets=nil,showanimation=true)
      if CableClub::DISABLE_SKETCH_ONLINE && !@battle.internalbattle
        @battle.pbDisplay(_INTL("But it failed!"))
        return -1
      end
      return _cc_pbEffect(attacker,opponent,hitnum,alltargets,showanimation)
    end
  end
end


class Socket
  def recv_up_to(maxlen, flags = 0)
    retString=""
    buf = "\0" * maxlen
    retval=Winsock.recv(@fd, buf, buf.size, flags)
    SocketError.check if retval == -1
    retString+=buf[0,retval]
    return retString
  end

  def write_ready?
    SocketError.check if (ret = Winsock.select(1, 0, [1, @fd].pack("ll"), 0, [0, 0].pack("ll"))) == -1
    return ret != 0
  end
end

class Connection
  class Disconnected < Exception; end
  class ProtocolError < StandardError; end

  def self.open(host, port)
    # XXX: Non-blocking connect.
    TCPSocket.open(host, port) do |socket|
      connection = Connection.new(socket)
      yield connection
    end
  end

  def initialize(socket)
    @socket = socket
    @recv_parser = Parser.new
    @recv_records = []
    @discard_records = 0
  end

  def update
    if @socket.ready?
      recvd = @socket.recv_up_to(4096, 0)
      raise Disconnected.new("server disconnected") if recvd.empty?
      @recv_parser.parse(recvd) {|record| @recv_records << record}
    end
    # Process at most one record so that any control flow in the block doesn't cause us to lose records.
    if !@recv_records.empty?
      record = @recv_records.shift
      if record.disconnect?
        reason = record.str() rescue "unknown error"
        raise Disconnected.new(reason)
      end
      if @discard_records == 0
        begin
          yield record
        else
          raise ProtocolError.new("Unconsumed input: #{record}") if !record.empty?
        end
      else
        @discard_records -= 1
      end
    end
  end

  def can_send?
    return @socket.write_ready?
  end

  def send
    # XXX: Non-blocking send.
    # but note we don't update often so we need some sort of drained?
    # for the send buffer so that we can delay starting the battle.
    writer = RecordWriter.new
    yield writer
    @socket.send(writer.line!)
  end

  def discard(n)
    raise "Cannot discard #{n} messages." if n < 0
    @discard_records += n
  end
end

class Parser
  def initialize
    @buffer = ""
  end

  def parse(data)
    return if data.empty?
    lines = data.split("\n", -1)
    lines[0].insert(0, @buffer)
    @buffer = lines.pop
    lines.each do |line|
      yield RecordParser.new(line) if !line.empty?
    end
  end
end

class RecordParser
  def initialize(data)
    @fields = []
    field = ""
    escape = false
    # each_char and chars don't exist.
    for i in (0...data.length)
      char = data[i].chr
      if char == "," && !escape
        @fields << field
        field = ""
      elsif char == "\\" && !escape
        escape = true
      else
        field += char
        escape = false
      end
    end
    @fields << field
    @fields.reverse!
  end

  def empty?; return @fields.empty? end

  def disconnect?
    if @fields.last == "disconnect"
      @fields.pop
      return true
    else
      return false
    end
  end

  def nil_or(t)
    raise Connection::ProtocolError.new("Expected nil or #{t}, got EOL") if @fields.empty?
    if @fields.last.empty?
      @fields.pop
      return nil
    else
      return self.send(t)
    end
  end

  def bool
    raise Connection::ProtocolError.new("Expected bool, got EOL") if @fields.empty?
    field = @fields.pop
    if field == "true"
      return true
    elsif field == "false"
      return false
    else
      raise Connection::ProtocolError.new("Expected bool, got #{field}")
    end
  end

  def int
    raise Connection::ProtocolError.new("Expected int, got EOL") if @fields.empty?
    field = @fields.pop
    begin
      return Integer(field)
    rescue
      raise Connection::ProtocolError.new("Expected int, got #{field}")
    end
  end

  def str
    raise Connection::ProtocolError.new("Expected str, got EOL") if @fields.empty?
    @fields.pop
  end

  def sym
    raise Connection::ProtocolError.new("Expected sym, got EOL") if @fields.empty?
    @fields.pop.to_sym
  end

  def to_s; @fields.reverse.join(", ") end
end

class RecordWriter
  def initialize
    @fields = []
  end

  def line!
    line = @fields.map {|field| escape!(field)}.join(",")
    line += "\n"
    @fields = []
    return line
  end

  def escape!(s)
    s.gsub!("\\", "\\\\")
    s.gsub!(",", "\,")
    return s
  end

  def nil_or(t, o)
    if o.nil?
      @fields << ""
    else
      self.send(t, o)
    end
  end

  def bool(b); @fields << b.to_s end
  def int(i); @fields << i.to_s end
  def str(s) @fields << s end
  def sym(s); @fields << s.to_s end
end

class MersenneTwisterRandom
  F = 1812433253
  W = 32
  N = 624
  M = 697
  R = 31
  A = 0x9908B0DF
  U = 11
  D = 0xFFFFFFFF
  S = 7
  B = 0x9D2C5680
  T = 15
  C = 0xEFC60000
  L = 18
  LOWER_MASK = (1<<R) - 1
  LOWER_W_MASK = (1<<W) - 1
  UPPER_MASK = (~LOWER_MASK) & LOWER_W_MASK
  
  attr_reader :seed
  
  def initialize(seed = nil)
    @index = N
    seed(seed)
  end
  
  def seed(seed = nil)
    @index = 0
    @seed = seed || rand(1<<W)
    @mt = []
    @mt[0] = @seed
    (1...N).each do |i|
      @mt[i] = (F * (@mt[i-1] ^ (@mt[i-1] >> (W-2))) + i) & LOWER_W_MASK
    end
    return @seed
  end
  
  def rand(a=0)
    a = a.to_i.abs
    ret = extract_number
    if a == 0
      return ret.to_f/(1<<W).to_f
    end
    return ret % a
  end
  
  def extract_number
    if @index >= N
      if @index > N
        raise "Generator not Seeded"
      end
      twist
    end
    y = @mt[@index]
    y = y ^ ((y >> U) & D)
    y = y ^ ((y << S) & B)
    y = y ^ ((y << T) & C)
    y = y ^ (y >> 1)
    
    @index = @index + 1
    return y & LOWER_W_MASK
  end
  
  def twist
    (0...N).each do |i|
      x  = (@mt[i] & UPPER_MASK) | (@mt[(i+1) % N] & LOWER_MASK)
      xA = x>> 1
      xA = xA ^ A if (x % 2)!=0
      @mt[i] = @mt[(i + M) % N] ^ xA
    end
    @index = 0
  end
end