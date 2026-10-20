module System
  VERSION="rsxp 0.1.0"
  START_TIME=Process.clock_gettime(Process::CLOCK_MONOTONIC)
  def self.uptime = Process.clock_gettime(Process::CLOCK_MONOTONIC)-START_TIME
  def self.real_uptime = uptime
  def self.platform = RUBY_PLATFORM
  # Host queries used by the Win32API shim and by cross-platform games.
  def self.is_windows? = !!(RUBY_PLATFORM =~ /mingw|mswin|cygwin/)
  def self.is_linux? = !!(RUBY_PLATFORM =~ /linux/)
  def self.is_mac? = !!(RUBY_PLATFORM =~ /darwin/)
  def self.puts(*messages) = Kernel.puts(*messages)
  def self.user_language = ENV.fetch('LANG','en_US').split(/[.@]/).first
  def self.user_name = ENV['USER'] || ENV['USERNAME'] || 'Player'
  def self.game_title = ENV.fetch('RSXP_GAME_TITLE','rsxp')
  def self.power_state
    seconds,percent,discharging = RSXP.call('system_power').split("\t")
    seconds,percent = seconds.to_i,percent.to_i
    {seconds: (seconds < 0 ? nil : seconds),percent: (percent < 0 ? nil : percent),discharging: discharging == '1'}
  end
  def self.set_window_title(title) = RSXP.call('window_title',payload:String(title))
  # mkxp-z System compatibility surface. Values that need OS/host support that
  # rsxp does not expose return safe neutral results instead of raising.
  def self.window_title = (@window_title || game_title)
  def self.window_title=(title)
    @window_title=title.to_s
    set_window_title(@window_title)
  end
  def self.delta = Graphics.delta
  def self.memory
    begin
      File.foreach('/proc/meminfo') do |line|
        if line =~ /\AMemTotal:\s+(\d+)\s+kB/
          return Regexp.last_match(1).to_i * 1024
        end
      end
    rescue StandardError
      # Not Linux, or /proc is unavailable.
    end
    0
  end
  def self.nproc
    begin
      require 'etc'
      Etc.nprocessors
    rescue StandardError
      1
    end
  end
  def self.file_exist?(path) = FileTest.exist?(path)
  def self.launch(*_args) = false
  def self.mount(*_args) = nil
  def self.unmount(*_args) = nil
  def self.reload_cache = nil
  def self.desensitize(path) = path.to_s
  def self.show_settings = nil
  def self.default_font_family=(value)
    @default_font_family=value
  end
  def self.parse_csv(text)
    begin
      require 'csv'
      CSV.parse(text.to_s)
    rescue StandardError, LoadError
      text.to_s.split(/\r?\n/).map { |line| line.split(',') }
    end
  end
  def self.is_bsd? = !!(RUBY_PLATFORM =~ /bsd/)
  def self.is_really_windows? = is_windows?
  def self.is_really_linux? = is_linux?
  def self.is_really_mac? = is_mac?
  def self.is_really_bsd? = is_bsd?
  def self.is_rosetta? = false
  def self.is_wine? = false
  def self.data_directory
    base=ENV['XDG_DATA_HOME'] || (ENV['HOME'] && File.join(ENV['HOME'],'.local','share')) || File.join(Dir.pwd,'.local','share')
    path=File.join(base,'rsxp',game_title.gsub(/[^[:alnum:]_. -]/,'_'))
    begin
      FileUtils.mkdir_p(path)
    rescue StandardError
      # The browser/WASI filesystem may be read-only; saves then fall back to
      # whatever the host provides.
    end
    path
  end
end

# mkxp-z configuration surface. rsxp has no user configuration file, so reads
# return nil (or the caller's default) and writes are kept for the session.
module CFG
  @values={}
  def self.[](key) = @values[key.to_s]
  def self.[]=(key,value)
    @values[key.to_s]=value
  end
end

