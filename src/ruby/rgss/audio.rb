# Host-backed audio API for the desktop Kira/CPAL backend and browser media
# backend. The host owns playback while Ruby retains the RGSS method surface.
module Audio
  # Desktop MRI installs these entry points through Magnus. Keep this bridge
  # implementation for mobile MRI and browser Ruby, which do not load them.
  unless singleton_class.method_defined?(:bgm_play)
  # Native ops; `channel` is one of bgm/bgs/me/se. Positions are seconds and
  # fade times are milliseconds, matching rsxp.
  def self.__play(channel, file, volume, pitch, position = 0.0)
    RSXP.call("audio_play\t#{channel}\t#{RSXP.volume(volume)}\t#{Float(pitch)}\t#{Float(position)}",
              payload: RSXP.utf8(file))
    nil
  end
  def self.bgm_play(file, volume=100, pitch=100, position=0, _track=nil)
    __play('bgm', file, volume, pitch, position)
  end
  def self.bgm_stop(_track=nil)
    RSXP.call('audio_stop','bgm')
    nil
  end
  def self.bgm_fade(time, _track=nil)
    RSXP.call('audio_fade','bgm',Integer(time))
    nil
  end
  def self.bgm_pos(_track=nil) = RSXP.call('audio_pos','bgm').to_f
  def self.bgm_volume = RSXP.call('audio_volume','bgm').to_i
  def self.bgm_set_volume(value)
    RSXP.call('audio_set_volume','bgm',RSXP.volume(value))
    nil
  end
  def self.bgs_play(file, volume=100, pitch=100, _position=0, _track=nil)
    __play('bgs', file, volume, pitch)
  end
  def self.bgs_stop(_track=nil)
    RSXP.call('audio_stop','bgs')
    nil
  end
  def self.bgs_fade(time, _track=nil)
    RSXP.call('audio_fade','bgs',Integer(time))
    nil
  end
  def self.bgs_pos(_track=nil) = RSXP.call('audio_pos','bgs').to_f
  def self.me_play(file, volume=100, pitch=100)
    __play('me', file, volume, pitch)
  end
  def self.me_stop(*_args)
    RSXP.call('audio_stop','me')
    nil
  end
  def self.me_fade(time, *_args)
    RSXP.call('audio_fade','me',Integer(time))
    nil
  end
  def self.se_play(file, volume=100, pitch=100)
    __play('se', file, volume, pitch)
  end
  def self.se_stop(*_args)
    RSXP.call('audio_stop','se')
    nil
  end
  def self.setup_midi = nil
  def self.__reset__
    %w[bgm bgs me se].each { |channel| RSXP.call('audio_stop',channel) }
    nil
  end
  end
end

RGSS_VERSION = RSXP.call('version').to_i
if RGSS_VERSION==1
  class Object
    alias_method :id, :__id__ unless method_defined?(:id)
  end
end
$DEBUG = false
$TEST = RGSS_VERSION >= 2
$BTEST = false

