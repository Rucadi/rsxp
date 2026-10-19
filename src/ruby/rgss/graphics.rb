module Graphics
  def self.fullscreen = RSXP.call('window_fullscreen')=='1'
  def self.fullscreen=(value)
    RSXP.call('window_fullscreen_set',value ? 1 : 0)
  end
  def self.scale = RSXP.call('window_scale').to_f
  def self.scale=(value)
    RSXP.call('window_scale_set',Float(value))
  end
  def self.center = RSXP.call('window_center')
  def self.show_cursor=(value)
    RSXP.call('window_cursor',value ? 1 : 0)
  end
  def self.freeze = RSXP.call('graphics_freeze')
  def self.transition(duration=8,filename='',vague=40)
    raise NotImplementedError,'pattern transitions are not ported yet' unless filename.to_s.empty?
    frames=[Integer(duration),0].max
    RSXP.call('graphics_transition',frames)
    frames.times {update}
    nil
  end
  def self.brightness = RSXP.call('graphics_brightness').to_i
  def self.brightness=(value)
    RSXP.call('graphics_brightness_set',RSXP.channel(value))
  end
  def self.fadeout(duration)
    frames=[Integer(duration),1].max
    start=brightness
    frames.times { |i| self.brightness=start*(frames-i-1)/frames;update }
  end
  def self.fadein(duration)
    frames=[Integer(duration),1].max
    start=brightness
    frames.times { |i| self.brightness=start+(255-start)*(i+1)/frames;update }
  end
  def self.frame_reset = RSXP.call('graphics_frame_reset')
  def self.width = (@__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i))[0]
  def self.height = (@__size ||= RSXP.call('graphics_size').split("\t").map!(&:to_i))[1]
  def self.frame_count = RSXP.call('graphics_frame').to_i
  def self.frame_count=(value)
    RSXP.call('graphics_frame_set',Integer(value))
  end
  def self.frame_rate = RSXP.call('graphics_rate').to_i
  def self.frame_rate=(value)
    RSXP.call('graphics_rate_set',Integer(value))
  end
  def self.resize_screen(width,height)
    result=RSXP.call('graphics_resize',Integer(width),Integer(height))
    @__size=nil
    result
  end
  # Resizes only the logical render target, not the native window. The host
  # scales the target to fit, so a larger target shows more of the scene at a
  # smaller scale. Used by the "map panoramic mode" runtime patch.
  def self.render_size=(size)
    RSXP.call('graphics_resolution_set',Integer(size[0]),Integer(size[1]))
    @__size=nil
    nil
  end
  def self.update
    now=Time.now.to_f
    if @last_update_time
      elapsed=now-@last_update_time
      if elapsed>0.0
        @delta=elapsed
        @average_frame_rate=(@average_frame_rate || frame_rate.to_f)*0.9+(1.0/elapsed)*0.1
      end
    end
    @last_update_time=now
    Viewport.__sync_all
    Plane.__sync_all
    Tilemap.__sync_all
    Sprite.__sync_all
    RSXP.call('graphics_update')
    nil
  end
  def self.wait(frames)
    [Integer(frames),0].max.times { update }
    nil
  end
  def self.snap_to_bitmap = Bitmap.__from_id(RSXP.call('graphics_snap'))
  # Real time since the previous update, in seconds (mkxp-z Graphics.delta).
  def self.delta = (@delta || 0.0)
  def self.average_frame_rate = (@average_frame_rate || frame_rate.to_f)
  def self.display_width = width
  def self.display_height = height
  def self.resize_window(width,height)
    resize_screen(Integer(width),Integer(height))
    nil
  end
  def self.screenshot(filename)
    bitmap=snap_to_bitmap
    begin
      bitmap.to_file(filename)
    ensure
      bitmap.dispose
    end
    nil
  end
  def self.frameskip = (@frameskip || false)
  def self.frameskip=(value)
    @frameskip=value ? true : false
  end
  def self.threadsafe = (@threadsafe || false)
  def self.threadsafe=(value)
    @threadsafe=value ? true : false
  end
  # Plays a Theora (.ogv) movie centred and scaled to the screen, blocking until
  # it ends or (when skippable) the player cancels. Video decodes in the engine
  # on every platform, so no system codec is required.
  def self.play_movie(filename,volume=100,skippable=false)
    info=begin
      RSXP.call('movie_open',Integer(volume),skippable ? 1 : 0,payload:String(filename))
    rescue StandardError
      nil
    end
    return if info.nil? || info.empty?
    id,width,height=info.split("\t").map!(&:to_i)
    return if id.to_i<=0
    width=[width,1].max
    height=[height,1].max
    bitmap=Bitmap.new(width,height)
    sprite=Sprite.new
    sprite.bitmap=bitmap
    sprite.z=5001
    ratio=[Graphics.width.to_f/width,Graphics.height.to_f/height].min
    sprite.zoom_x=ratio
    sprite.zoom_y=ratio
    sprite.x=(Graphics.width/2)-(width*ratio/2)
    sprite.y=(Graphics.height/2)-(height*ratio/2)
    begin
      loop do
        status=RSXP.call('movie_frame',id,bitmap.__id).to_s
        update
        break if status=='0'
        break if skippable && (Input.trigger?(Input::C) || Input.trigger?(Input::B))
      end
    ensure
      sprite.dispose
      bitmap.dispose
      RSXP.call('movie_close',id)
    end
    nil
  end
end

