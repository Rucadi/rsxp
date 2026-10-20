# SPDX-License-Identifier: GPL-2.0-or-later
#
# In-memory compatibility for the Cable Club client's per-turn RNG
# synchronization. Some Essentials Cable Club plugins call `srand` on their
# Random instance, but that method is a private Kernel method in Ruby. Their
# battle RNG can also be nil when another battle plugin replaces initialization.
# This adapter is limited to Battle_CableClub and does not alter game files.

module CableClubRngCompat
  def self.enabled? = @enabled == true

  module RandomInstance
    def srand(seed = (Random.new_seed & 0x7fffffff))
      @rsxp_cable_club_random = Random.new(seed)
      seed
    end

    def rand(*arguments)
      return @rsxp_cable_club_random.rand(*arguments) if CableClubRngCompat.enabled? && @rsxp_cable_club_random
      super
    end
  end

  module BattleAccessor
    def battleRNG
      return super unless CableClubRngCompat.enabled?
      rng = instance_variable_get(:@battleRNG)
      unless rng
        rng = Random.new(Random.new_seed & 0x7fffffff)
        instance_variable_set(:@battleRNG, rng)
      end
      singleton = rng.singleton_class
      singleton.prepend(RandomInstance) unless singleton.ancestors.include?(RandomInstance)
      rng
    end
  end

  def self.apply
    @enabled = true
    if defined?(Battle_CableClub) && !Battle_CableClub.ancestors.include?(BattleAccessor)
      Battle_CableClub.prepend(BattleAccessor)
    end
  end

  def self.revert
    @enabled = false
  end
end

{ apply: CableClubRngCompat.method(:apply),
  revert: CableClubRngCompat.method(:revert) }
